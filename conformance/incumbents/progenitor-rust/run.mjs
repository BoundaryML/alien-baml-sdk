#!/usr/bin/env node
// Runs the Platform and Manager conformance scenarios through Alien's current
// Rust clients (alien-platform-api and alien-manager-api, Progenitor,
// `InterfaceStyle::Builder`) and writes how each behaves to
// conformance/baseline/progenitor-rust-<variant>/. Build the incumbents first:
// ALIEN_REF=/path/to/alien bench/build.sh
//
//   node conformance/incumbents/progenitor-rust/run.mjs [--phase N] [--variant default|full-api]
//
// Both variants by default: `default` is the Platform client as Alien builds
// it for its own code (only the operations in its openapi_filter.rs), and
// `full-api` is the crate's `full-api` feature (every operation). The Manager
// client has no such feature and is the same in both.
//
// For each variant, run.mjs builds the clients, reads the code Progenitor
// generated for them (`codegen.rs`), writes the operation table the Rust
// program includes (one row per operation: each parameter set through its
// builder method, with the type that method takes), and builds the program
// (src/main.rs). Everything built goes to .cache/incumbents/progenitor/.
//
// Scenarios are judged as for the Speakeasy adapter (../judge.mjs). The
// program's HTTP client sends to a recording proxy, which records each
// request and forwards the request the scenario expects to the stub, so the
// scripted response comes back whatever the client sent. Errors are reduced
// to {code, retryable, http_status} as Alien's own code reads them
// (`convert_sdk_error`, which `into_sdk_error()` calls).
import { execFileSync, spawn } from "node:child_process"
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { createServer } from "node:http"
import { dirname, join } from "node:path"
import { createInterface } from "node:readline"
import { fileURLToPath } from "node:url"
import { Baseline, camel, judge, loadScenarios, oracle, oracleEntry, repo, service, startStub } from "../judge.mjs"

const here = dirname(fileURLToPath(import.meta.url))
const manifest = join(here, "Cargo.toml")
const alien = join(repo, ".cache", "incumbents", "alien")
const work = join(repo, ".cache", "incumbents", "progenitor")
if (!existsSync(join(alien, "client-sdks", "platform", "rust"))) throw new Error(`no Alien checkout in ${alien} (ALIEN_REF=… bench/build.sh)`)
const commit = readFileSync(join(repo, ".cache", "incumbents", "alien.commit"), "utf8").trim()

const variantArg = process.argv.indexOf("--variant")
const variants = variantArg > 0 ? [process.argv[variantArg + 1]] : ["default", "full-api"]
for (const v of variants) if (!["default", "full-api"].includes(v)) throw new Error(`unknown variant ${v}`)
const scenarios = loadScenarios(process.argv)

// ---------------------------------------------------------------------------
// Building: the clients, the operation table, the program

const cargoEnv = { ...process.env, CARGO_TARGET_DIR: join(work, "target") }

/**
 * `cargo build` of the program for `variant` with the table `table`; the
 * build's messages (JSON).
 */
function cargoBuild(variant, table) {
  const features = variant === "full-api" ? ["--features", "full-api"] : []
  return execFileSync("cargo", ["build", "--quiet", "--manifest-path", manifest, ...features, "--message-format=json-render-diagnostics"], {
    env: { ...cargoEnv, PROGENITOR_TABLE: table }, maxBuffer: 1 << 28, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"],
  }).split("\n").filter(Boolean).map((line) => JSON.parse(line))
}

/**
 * Builds the two clients and returns each one's generated code. The program
 * is built with an empty table first (Cargo takes features for the
 * workspace's own package only), which runs the clients' build scripts.
 */
function generatedCode(variant, table) {
  writeFileSync(table, ["platform", "manager"].map((svc) => `async fn ${svc}_call(_: &${svc}::Client, op: &str, _: &mut Args) -> Result<Value, Value> {\n    Err(json!({ "harness": format!("no table yet: {op}") }))\n}\n`).join("\n"))
  const outDirs = {}
  for (const message of cargoBuild(variant, table)) {
    if (message.reason !== "build-script-executed") continue
    for (const [svc, crate] of [["platform", "alien-platform-api"], ["manager", "alien-manager-api"]]) {
      if (message.package_id.includes(`#${crate}@`) || message.package_id.includes(`/${crate}#`)) outDirs[svc] = message.out_dir
    }
  }
  for (const svc of ["platform", "manager"]) if (!outDirs[svc]) throw new Error(`cargo did not report the generated code of the ${svc} client`)
  return Object.fromEntries(Object.entries(outDirs).map(([svc, dir]) => [svc, readFileSync(join(dir, "codegen.rs"), "utf8")]))
}

/** The text of the `<…>` that starts at `text[open]`. */
function angled(text, open) {
  let depth = 0
  for (let i = open; i < text.length; i++) {
    if (text[i] === "<") depth++
    else if (text[i] === ">" && text[i - 1] !== "-") {
      if (--depth === 0) return text.slice(open + 1, i)
    }
  }
  throw new Error(`unbalanced <> at ${open}`)
}

/** Splits `a, b<c, d>` at its top-level commas. */
function topLevel(text) {
  const parts = []
  let depth = 0, start = 0
  for (let i = 0; i < text.length; i++) {
    if (text[i] === "<") depth++
    else if (text[i] === ">" && text[i - 1] !== "-") depth--
    else if (text[i] === "," && depth === 0) {
      parts.push(text.slice(start, i))
      start = i + 1
    }
  }
  parts.push(text.slice(start))
  return parts.map((p) => p.replace(/\s+/g, " ").trim()).filter(Boolean)
}

/**
 * The operations of one client's generated code, by `METHOD path`: the
 * client method, each builder method and the type it takes, and what `send`
 * returns.
 */
function parseClient(code) {
  const split = code.indexOf("\npub mod builder {")
  if (split < 0) throw new Error("no builder module: not InterfaceStyle::Builder?")
  const methods = new Map([...code.slice(0, split).matchAll(/pub fn (\w+)\(\s*&self,?\s*\) -> builder::(\w+)<'_>/g)].map((m) => [m[2], m[1]]))
  const builder = code.slice(split)
  const ops = new Map()
  for (const block of builder.matchAll(/\n    impl<'a> (\w+)<'a> \{\n(.*?)\n    \}\n/gs)) {
    const [, struct, body] = block
    const route = /Sends a `(\w+)` request to `([^`]+)`/.exec(body)
    if (!route) continue
    const setters = []
    for (const m of body.matchAll(/pub fn (\w+)<V>\(mut self, value: V\) -> Self\s+where\s+V: std::convert::TryInto</g)) {
      setters.push({ name: m[1], type: angled(body, m.index + m[0].length - 1).replace(/\s+/g, " ").trim() })
    }
    const send = /pub async fn send\(\s*self,?\s*\)\s*->\s*Result</.exec(body)
    if (!send) throw new Error(`${struct}: no send()`)
    const [response, error] = topLevel(angled(body, send.index + send[0].length - 1))
    const method = methods.get(struct)
    if (!method) throw new Error(`${struct}: no client method returns it`)
    ops.set(`${route[1]} ${route[2]}`, { method, setters, response, error })
  }
  return ops
}

/** One table function (`platform_call` or `manager_call`). */
function tableFunction(svc, ops) {
  const crate = svc === "platform" ? "platform" : "manager"
  const rust = (type) => {
    if (/\bsuper::/.test(type)) throw new Error(`unexpected path in ${type}`)
    return type.replace(/(?<![\w:])types::/g, `${crate}::types::`)
  }
  const rows = []
  for (const op of [...ops.values()].sort((a, b) => a.method.localeCompare(b.method))) {
    let finish
    if (svc === "manager") {
      if (op.error !== "Error<()>") throw new Error(`manager ${op.method}: unexpected error type ${op.error}`)
      finish = "manager_outcome(b.send().await).await"
    } else if (op.error === "Error<()>") {
      finish = "platform_untyped_outcome(b.send().await).await"
    } else if (op.error !== "Error<types::ApiError>") {
      throw new Error(`platform ${op.method}: unexpected error type ${op.error}`)
    } else if (op.response === "ResponseValue<ByteStream>") {
      finish = "platform_stream_outcome(b.send().await).await"
    } else {
      finish = "platform_outcome(b.send().await)"
    }
    const sets = op.setters.map((s) => s.name === "body"
      ? `            if let Some(v) = a.body::<${rust(s.type)}>()? { b = b.body(v); }\n`
      : `            if let Some(v) = a.take::<${rust(s.type)}>(${JSON.stringify(s.name)})? { b = b.${s.name}(v); }\n`)
    rows.push(`        ${JSON.stringify(op.method)} => {\n            let mut b = c.${op.method}();\n${sets.join("")}            a.finish()?;\n            Ok(${finish})\n        }\n`)
  }
  return `#[allow(unused_mut)]\nasync fn ${svc}_call(c: &${crate}::Client, op: &str, a: &mut Args) -> Result<Value, Value> {\n    match op {\n${rows.join("")}        other => Err(json!({ "harness": format!("the ${svc} client has no method {other}") })),\n    }\n}\n`
}

/** Builds the program for `variant`; returns its path and each client's operations. */
function build(variant) {
  const dir = join(work, variant)
  mkdirSync(dir, { recursive: true })
  const table = join(dir, "table.rs")
  const code = generatedCode(variant, table)
  const clients = { platform: parseClient(code.platform), manager: parseClient(code.manager) }
  writeFileSync(table, `// Generated by conformance/incumbents/progenitor-rust/run.mjs from Alien ${commit} (${variant}); do not edit.\n\n${tableFunction("platform", clients.platform)}\n${tableFunction("manager", clients.manager)}`)
  const built = cargoBuild(variant, table).filter((m) => m.reason === "compiler-artifact" && m.target?.name === "progenitor_conformance" && m.executable)
  if (built.length !== 1) throw new Error("cargo did not report the program it built")
  const program = join(dir, "progenitor_conformance")
  copyFileSync(built[0].executable, program)
  return { program, clients }
}

// ---------------------------------------------------------------------------
// Calling the incumbent

/** The program, as an async function from a call to its answer. */
function startProgram(path) {
  const child = spawn(path, [], { stdio: ["pipe", "pipe", "inherit"] })
  const lines = createInterface({ input: child.stdout })
  const waiting = []
  lines.on("line", (line) => waiting.shift()(JSON.parse(line)))
  return {
    call: (request) => new Promise((ok) => {
      waiting.push(ok)
      child.stdin.write(JSON.stringify(request) + "\n")
    }),
    stop: () => child.stdin.end(),
  }
}

/**
 * The recording proxy the clients send to: it records each request and
 * forwards the request the scenario expects to the stub instead (an
 * unexpected one as it is, so the stub refuses it), then relays the answer.
 * A connection the stub drops is dropped.
 */
async function startProxy(stubBase) {
  let current = { exchanges: [], captured: [] }
  const server = createServer(async (req, res) => {
    const chunks = []
    for await (const chunk of req) chunks.push(chunk)
    const body_text = Buffer.concat(chunks).toString("utf8")
    let json
    try { json = body_text ? JSON.parse(body_text) : undefined } catch { json = undefined }
    const { exchanges, captured } = current
    captured.push({ method: req.method, path: req.url, headers: req.headers, body_text, json })
    const expect = exchanges[captured.length - 1]?.expect
    try {
      const answer = expect
        ? await fetch(stubBase + expect.path, { method: expect.method, headers: { ...(expect.headers ?? {}) }, body: "json" in expect ? JSON.stringify(expect.json) : undefined })
        : await fetch(stubBase + req.url, { method: req.method, body: body_text || undefined })
      const body = Buffer.from(await answer.arrayBuffer())
      const headers = Object.fromEntries([...answer.headers].filter(([name]) => !["content-length", "transfer-encoding", "connection", "content-encoding", "keep-alive"].includes(name)))
      res.writeHead(answer.status, headers)
      res.end(body)
    } catch {
      res.socket?.destroy()
    }
  })
  await new Promise((ok) => server.listen(0, "127.0.0.1", ok))
  return {
    base: `http://127.0.0.1:${server.address().port}`,
    expect: (exchanges) => (current = { exchanges, captured: [] }).captured,
    stop: () => server.close(),
  }
}

// Phase 1 scenarios that name a function instead of an operation: the operation and its arguments.
const namedCalls = {
  "platform.createSetupLink": (a) => ({ op: "platform.setupLinks.create", args: { client: a.client, body: a.request } }),
  "platform.getProjectTemplateUrls": (a) => ({ op: "platform.projects.getTemplateUrls", args: { client: a.client, idOrName: a.idOrName } }),
  "platform.bootstrapCommands": (a) => ({ op: "platform.commands.bootstrap", args: { client: a.client, body: a.request } }),
  "platform.getManager": (a) => ({ op: "platform.managers.get", args: { client: a.client, id: a.id } }),
  "manager.getCommandStatus": (a) => ({ op: "manager.commands.getCommandStatus", args: { client: a.client, commandId: a.command_id } }),
}

/** Progenitor's name for a wire name, compared loosely: `If-Match` and `if_match`, `idOrName` and `id_or_name`. */
const loose = (name) => name.replace(/[^A-Za-z0-9]/g, "").toLowerCase()

/**
 * The program's call for a scenario's operation (`op` and `args` by BAML
 * names), or `{skip}` when the client does not have the operation.
 */
function incumbentCall(clients, op, args, proxy, stubBase) {
  const [svc, ...rest] = op.split(".")
  const method = rest.at(-1)
  const group = rest.length > 1 ? rest[0] : null
  const entry = oracleEntry(svc, group, method)
  if (!entry) throw new Error(`${op}: not in the naming oracle`)
  const target = clients[svc].get(`${entry.method} ${entry.path}`)
  if (!target) return { skip: `not in this client (${entry.id})` }
  // BAML argument name → wire name → the builder method.
  const wireOf = Object.fromEntries(Object.entries(oracle.operations[svc][entry.id]?.param_names ?? {}).map(([wire, name]) => [name, wire]))
  const params = {}
  for (const [name, value] of Object.entries(args)) {
    if (name === "client" || name === "body") continue
    const wire = wireOf[name] ?? name
    const setter = target.setters.find((s) => s.name !== "body" && loose(s.name) === loose(wire))
    if (!setter) throw new Error(`${op}: the client's ${target.method} has no builder method for \`${wire}\``)
    params[setter.name] = value
  }
  // `workspace` is an ordinary parameter of the Rust client; the BAML SDKs send the client's.
  if (args.client.workspace && target.setters.some((s) => s.name === "workspace")) params.workspace = args.client.workspace
  return {
    request: {
      service: svc,
      op: target.method,
      base_url: args.client.base_url.replace(stubBase, proxy.base),
      bearer: (svc === "platform" ? args.client.api_key : args.client.token) || null,
      timeout_ms: args.client.timeout_ms ?? null,
      params,
      ...("body" in args ? { body: args.body } : {}),
    },
  }
}

const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v)

/**
 * The client's response, serialized again, as the BAML SDKs present it: the
 * serialization writes wire names, so a field the BAML SDKs rename
 * (`custom_domains` → `customDomains`, `test` → `test_`) is matched to its
 * BAML name, and the unlisted keys a BAML class keeps under
 * `additionalProperties` are looked for at the top level, where Progenitor's
 * flattened map writes them. Only names the expected result has are moved.
 */
function hostView(want, got) {
  if (Array.isArray(want) && Array.isArray(got)) return got.map((g, i) => hostView(want[i], g))
  if (!isObject(want) || !isObject(got)) return got
  const out = {}
  for (const [key, value] of Object.entries(got)) {
    const name = [camel(key), `${key}_`].find((n) => n !== key && n in want && !(key in want))
    out[name ?? key] = value
  }
  if (isObject(want.additionalProperties) && !("additionalProperties" in out)) {
    const extra = Object.keys(want.additionalProperties).filter((k) => k in out && !(k in want))
    if (extra.length) out.additionalProperties = Object.fromEntries(extra.map((k) => [k, out[k]]))
    for (const k of extra) delete out[k]
  }
  for (const key of Object.keys(out)) out[key] = hostView(want[key], out[key])
  return out
}

/**
 * The program's answer as an outcome (judge.mjs). An error's context is the
 * server's (request ID, hint, and the rest as `details`) only when it came
 * from an error body; otherwise it is the client's own (`url`, `status`).
 */
function outcomeOf(answer, op, scenario) {
  if ("harness" in answer) throw new Error(`${op}: ${answer.harness}`)
  if ("ok" in answer) return { ok: hostView(scenario.expect_result?.ok ?? null, answer.ok) }
  if ("rejected" in answer) {
    const { argument, message } = answer.rejected
    return { error: { code: "(rejected by the client's types)", http_status: null, message: `${argument}: ${message}` } }
  }
  const e = answer.error
  const fromBody = e.variant === "ErrorResponse" || (e.variant === "UnexpectedResponse" && e.code !== "UNEXPECTED_RESPONSE")
  const { requestId = null, ...rest } = fromBody && isObject(e.context) ? e.context : {}
  const context = { requestId, hint: e.hint ?? null, details: Object.keys(rest).length ? JSON.stringify(rest) : null }
  return { error: { code: e.code, retryable: e.retryable ?? null, http_status: e.http_status ?? null, message: e.message, variant: e.variant, context } }
}

// ---------------------------------------------------------------------------
// Run

// Build first: a connection to the stub left idle through a long build goes stale.
const builds = new Map(variants.map((variant) => [variant, build(variant)]))
const stub = await startStub()
const proxy = await startProxy(stub.base)
for (const [variant, { program, clients }] of builds) {
  const incumbent = startProgram(program)
  const baseline = new Baseline(join(repo, "conformance", "baseline", `progenitor-rust-${variant}`), stub.base, proxy.base)
  for (const raw of scenarios) {
    const svc = service(raw)
    // The scenario with its templates resolved by the stub (secrets, `${now…}`).
    const { call, scenario } = await stub.post("/__load", { scenario: raw })
    let named = call.op ? { op: call.op, args: call.args } : namedCalls[call.fn]?.(call.args)
    if (!named) {
      baseline.skip(scenario.id, `no incumbent call for ${call.fn}`)
      continue
    }
    const plan = incumbentCall(clients, named.op, named.args, proxy, stub.base)
    if (plan.skip) {
      baseline.skip(scenario.id, plan.skip)
      continue
    }
    const captured = proxy.expect(scenario.exchanges ?? [])
    const outcome = outcomeOf(await incumbent.call(plan.request), named.op, scenario)
    baseline.record(scenario.id, outcome, judge(scenario, svc, captured, outcome))
  }
  incumbent.stop()
  const count = (svc) => clients[svc].size
  baseline.summary([
    `# Incumbent baseline: Progenitor Rust (${variant})`,
    "",
    "Generated by `conformance/incumbents/progenitor-rust/run.mjs`: the Platform and Manager scenarios run through",
    `\`alien-platform-api\` (${variant === "full-api" ? "the `full-api` feature" : "default features"}, ${count("platform")} operations) and \`alien-manager-api\` (${count("manager")} operations) from Alien ${commit.slice(0, 7)} against the same stub.`,
    "\"Request\" compares what the client sent with what the scenario expects (the BAML SDKs send exactly that);",
    "\"Outcome\" compares the decoded response, serialized again, or the error, as `convert_sdk_error` reads it, with what",
    "the BAML SDKs return. Equivalent differences (a spec default filled in, a date-time written with milliseconds, query",
    "parameters in another order) do not break parity.",
  ], "Skipped (operation not in this client)")
}
proxy.stop()
stub.stop()
