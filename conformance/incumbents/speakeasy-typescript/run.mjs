#!/usr/bin/env node
// Runs the Platform and Manager conformance scenarios through Alien's current
// TypeScript SDKs (@alienplatform/platform-api and @alienplatform/manager-api,
// Speakeasy) and writes how each behaves to
// conformance/baseline/speakeasy-typescript/. Build the SDKs first:
// ALIEN_REF=/path/to/alien bench/build.sh
//
//   node conformance/incumbents/speakeasy-typescript/run.mjs [--phase N]
//
// INCUMBENTS_DIR overrides where the built SDKs are (default .cache/incumbents).
//
// Each scenario is judged twice, request and outcome (../judge.mjs). Errors
// are reduced as for the BAML SDKs: an API error body (Platform APIError,
// Manager ErrorResponse) to {code, retryable, http_status} and the context it
// carries; any other error to {code: "(<ErrorClass>)", http_status}.
import { existsSync, readFileSync } from "node:fs"
import { join } from "node:path"
import { Baseline, camel, judge, loadScenarios, normalize, oracleEntry, repo, service, startStub } from "../judge.mjs"

const incumbents = process.env.INCUMBENTS_DIR || join(repo, ".cache", "incumbents")

async function loadSdk(name) {
  const esm = join(incumbents, name, "esm")
  if (!existsSync(join(esm, "index.js"))) return null
  const pkg = JSON.parse(readFileSync(join(incumbents, name, "package.json"), "utf8"))
  return {
    name: pkg.name,
    version: pkg.version,
    root: await import(join(esm, "index.js")),
    errors: await import(join(esm, "models", "errors", "index.js")),
    models: await import(join(esm, "models", "index.js")),
    operations: await import(join(esm, "models", "operations", "index.js")),
    http: await import(join(esm, "lib", "http.js")),
  }
}
const sdks = { platform: await loadSdk("platform-api"), manager: await loadSdk("manager-api") }
if (!sdks.platform) throw new Error(`no @alienplatform/platform-api build in ${incumbents} (ALIEN_REF=… bench/build.sh)`)

const scenarios = loadScenarios(process.argv)

// ---------------------------------------------------------------------------
// Calling the incumbent

const stub = await startStub()
const { base, post } = stub

/**
 * An HTTP client that records each request the SDK makes and sends the stub
 * the request the scenario expects instead, so the scripted response comes
 * back whatever the SDK sent.
 */
function recordingClient(svc, exchanges, captured) {
  return new sdks[svc].http.HTTPClient({
    fetcher: async (request) => {
      const body_text = request.body ? await request.text() : ""
      let json
      try { json = body_text ? JSON.parse(body_text) : undefined } catch { json = undefined }
      const url = new URL(request.url)
      captured.push({ method: request.method, path: url.pathname.replace(new URL(base).pathname.replace(/\/$/, ""), "") + url.search, headers: Object.fromEntries(request.headers), body_text, json })
      const expect = exchanges[captured.length - 1]?.expect
      if (!expect) return fetch(request.url, { method: request.method, headers: request.headers, body: body_text || undefined, signal: request.signal })
      const headers = { ...(expect.headers ?? {}) }
      return fetch(base + expect.path, { method: expect.method, headers, body: "json" in expect ? JSON.stringify(expect.json) : undefined, signal: request.signal })
    },
  })
}

function platformClient(c, httpClient) {
  return new sdks.platform.root.Alien({ apiKey: c.api_key, serverURL: c.base_url, httpClient, ...(c.workspace ? { workspace: c.workspace } : {}), ...(c.timeout_ms ? { timeoutMs: c.timeout_ms } : {}) })
}
function managerClient(c, httpClient) {
  return new sdks.manager.root.AlienManager({ serverURL: c.base_url, httpClient, ...(c.token ? { bearer: c.token } : {}), ...(c.timeout_ms ? { timeoutMs: c.timeout_ms } : {}) })
}

// zod v4 schemas: unwrap optional/nullable/default/pipe/lazy wrappers.
function unwrap(schema) {
  for (;;) {
    const def = schema?._zod?.def
    if (!def) return schema
    if (["optional", "nullable", "default", "prefault", "readonly", "catch", "nonoptional"].includes(def.type)) schema = def.innerType
    else if (def.type === "pipe") schema = def.in
    else if (def.type === "lazy") schema = def.getter()
    else return schema
  }
}

/** `value` (JSON) as the SDK takes it: date-time fields as `Date`, by the SDK's own outbound schema. */
function prepare(schema, value) {
  if (value === null || value === undefined || !schema) return value
  const s = unwrap(schema)
  const def = s?._zod?.def
  if (!def) return value
  switch (def.type) {
    case "date":
      return typeof value === "string" ? new Date(value) : value
    case "object": {
      if (typeof value !== "object" || Array.isArray(value)) return value
      const out = { ...value }
      for (const [key, sub] of Object.entries(def.shape)) if (key in out) out[key] = prepare(sub, out[key])
      return out
    }
    case "array":
      return Array.isArray(value) ? value.map((v) => prepare(def.element, v)) : value
    case "record":
      return typeof value === "object" ? Object.fromEntries(Object.entries(value).map(([k, v]) => [k, prepare(def.valueType, v)])) : value
    case "union": {
      for (const option of def.options) {
        const prepared = prepare(option, value)
        if (option.safeParse(prepared).success) return prepared
      }
      return value
    }
    default:
      return value
  }
}

function outboundSchema(svc, type) {
  const [kind, name] = type.split(".")
  const module = kind === "operations" ? sdks[svc].operations : sdks[svc].models
  return module[`${name}$outboundSchema`] ?? null
}

function shapeOf(schema) {
  const s = unwrap(schema)
  return s?._zod?.def?.shape ?? null
}

/** The incumbent call for a generated operation (`call.op`), from the naming oracle's call shape. */
function incumbentCall(loaded, httpClient) {
  const svc = service({ call: loaded })
  const [, ...rest] = loaded.op.split(".")
  const method = rest.at(-1)
  const group = rest.length > 1 ? rest[0] : null
  const entry = oracleEntry(svc, group, method)
  if (!entry) return { skip: "not in the incumbent SDK (the spec has it, the SDK's sources do not)" }
  if (!sdks[svc]) return { skip: `${svc} SDK not built` }
  const a = loaded.args
  const client = svc === "manager" ? managerClient(a.client, httpClient) : platformClient(a.client, httpClient)
  const target = group === null ? client : client[group]
  const fn = target?.[entry.method_name]
  if (typeof fn !== "function") return { skip: `the incumbent build has no ${group ?? ""}.${entry.method_name}` }
  const paramArgs = Object.fromEntries(loaded.params.filter((p) => p.name !== "body" && p.name in a).map((p) => [camel(p.name), a[p.name]]))
  let args
  if (entry.params.length === 0) {
    args = []
  } else if (entry.params.length === 1 && (entry.params[0].type.startsWith("models.") || loaded.params.every((p) => p.name === "body"))) {
    // The request is the body (`models.X`, or `operations.<OpId>Request` for an inline body).
    args = [prepare(outboundSchema(svc, entry.params[0].type), a.body)]
  } else if (entry.params.length === 1) {
    const schema = outboundSchema(svc, entry.params[0].type)
    const shape = shapeOf(schema) ?? {}
    const bodyKey = Object.keys(shape).find((k) => !(k in paramArgs) && !loaded.params.some((p) => camel(p.name) === k))
    const request = { ...paramArgs }
    if (bodyKey && "body" in a) request[bodyKey] = a.body
    args = [prepare(schema, request)]
  } else {
    // Two positional arguments (`x-speakeasy-max-method-params: 2`): a parameter
    // and the body, in the SDK's order (usually `(id, body)`; `(requestBody,
    // projectId)` for `webhooks.createWebhookEndpoint`).
    const isBody = (p) => p.name === "requestBody" || /^(models|operations)\./.test(p.type)
    if (entry.params.filter(isBody).length !== 1) throw new Error(`${loaded.op}: cannot tell which of the SDK's two arguments is the body`)
    args = entry.params.map((p) => (isBody(p) ? prepare(outboundSchema(svc, p.type), a.body) : paramArgs[camel(p.name)] ?? a[p.name]))
  }
  return { run: () => fn.apply(target, args) }
}

// Phase 1 scenarios that name a function instead of an operation.
const withDates = (request) => (request.expiresAt ? { ...request, expiresAt: new Date(request.expiresAt) } : request)
const namedCalls = {
  "platform.createSetupLink": (a, h) => platformClient(a.client, h).setupLinks.create(withDates(a.request)),
  "platform.getProjectTemplateUrls": (a, h) => platformClient(a.client, h).projects.getTemplateUrls({ idOrName: a.idOrName }),
  "platform.bootstrapCommands": (a, h) => platformClient(a.client, h).commands.bootstrap(a.request),
  "platform.getManager": (a, h) => platformClient(a.client, h).managers.get({ id: a.id }),
  "manager.getCommandStatus": (a, h) => managerClient(a.client, h).commands.getCommandStatus({ commandId: a.command_id }),
}

function reduce(svc, error) {
  const errors = sdks[svc].errors
  if (svc === "platform" && error instanceof errors.APIError) {
    const context = { requestId: error.requestId ?? null, hint: error.hint ?? null, details: error.context == null ? null : JSON.stringify(error.context) }
    return { code: error.code, retryable: error.retryable ?? false, http_status: error.statusCode, message: error.message, context }
  }
  if (svc === "manager" && error instanceof errors.ErrorResponse) {
    return { code: error.code, retryable: false, http_status: error.statusCode, message: error.message, context: { details: error.details ?? null } }
  }
  if (svc === "manager" && error instanceof errors.AlienError && typeof error.code === "string") {
    // The operations that declare Alien's error body.
    return { code: error.code, retryable: error.retryable ?? false, http_status: error.statusCode, message: error.message }
  }
  // A validation error says which value failed: its zod issues, shortened.
  const detail = typeof error?.pretty === "function" ? error.pretty().split("\n").slice(1).join(" ").replace(/\s+/g, " ").trim().slice(0, 300) : undefined
  return { code: `(${error?.constructor?.name ?? "unknown"})`, http_status: error?.statusCode ?? null, message: String(error?.message ?? error).split("\n")[0], ...(detail ? { detail } : {}) }
}

// ---------------------------------------------------------------------------
// Run

const baseline = new Baseline(join(repo, "conformance", "baseline", "speakeasy-typescript"), base)
for (const raw of scenarios) {
  const svc = service(raw)
  // The scenario with its templates resolved by the stub (secrets, `${now…}`).
  const { call, scenario } = await post("/__load", { scenario: raw })
  const captured = []
  const httpClient = sdks[svc] ? recordingClient(svc, scenario.exchanges ?? [], captured) : null
  let plan
  if (call.op) plan = incumbentCall(call, httpClient)
  else if (namedCalls[call.fn]) plan = { run: () => namedCalls[call.fn](call.args, httpClient) }
  else plan = { skip: `no incumbent call for ${call.fn}` }
  if (plan.skip) {
    baseline.skip(scenario.id, plan.skip)
    continue
  }
  let outcome
  try {
    outcome = { ok: normalize(await plan.run()) }
  } catch (error) {
    outcome = { error: reduce(svc, error) }
  }
  baseline.record(scenario.id, outcome, judge(scenario, svc, captured, outcome))
}
stub.stop()

baseline.summary([
  "# Incumbent baseline: Speakeasy TypeScript",
  "",
  "Generated by `conformance/incumbents/speakeasy-typescript/run.mjs`: the Platform and Manager scenarios run through",
  `\`${sdks.platform.name}\` ${sdks.platform.version}` + (sdks.manager ? ` and \`${sdks.manager.name}\` ${sdks.manager.version}` : "") + " against the same stub.",
  "\"Request\" compares what the SDK sent with what the scenario expects (the BAML SDKs send exactly that);",
  "\"Outcome\" compares the decoded response or error with what the BAML SDKs return. Equivalent differences",
  "(a spec default filled in, a date-time written with milliseconds, query parameters in another order) do not break parity.",
], "Skipped (operation not in the incumbent SDK)")
