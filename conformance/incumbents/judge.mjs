// What every incumbent adapter shares (conformance/incumbents/README.md):
// the scenarios, the specs, the stub, and how a scenario is judged and
// written to conformance/baseline/<adapter>/.
//
// Each scenario is judged twice:
// - Request parity: method, path, query, the headers the scenario names, and
//   the JSON body, against what the BAML SDKs send. A body field or query
//   parameter the incumbent fills with its spec default counts as equal to
//   the field left out (the BAML SDKs leave unset fields out), and so do a
//   date-time written with milliseconds, query parameters in another order
//   and `content-type` sent with no body.
// - Outcome parity: the decoded response, or the error, against what the
//   BAML SDKs return. To judge it whatever the request was, the stub always
//   receives the request the scenario expects.
import { spawn } from "node:child_process"
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { createInterface } from "node:readline"
import { fileURLToPath } from "node:url"
import { isDeepStrictEqual } from "node:util"

export const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..")

/** The service (`platform` or `manager`) a scenario calls. */
export const service = (s) => (s.call.op ?? s.call.fn).split(".")[0]

/** The Platform and Manager scenarios up to `--phase N` (all without it), sorted by id. */
export function loadScenarios(argv) {
  const phaseArg = argv.indexOf("--phase")
  const maxPhase = phaseArg > 0 ? Number(argv[phaseArg + 1]) : Number.POSITIVE_INFINITY
  const scenarioDir = join(repo, "conformance", "scenarios")
  return readdirSync(scenarioDir)
    .filter((f) => f.endsWith(".json"))
    .sort()
    .map((f) => JSON.parse(readFileSync(join(scenarioDir, f), "utf8")))
    .filter((s) => s.phase <= maxPhase && s.call && ["platform", "manager"].includes(service(s)))
}

// Alien's TypeScript SDK names (tools/openapi2baml/tests/naming_oracle.rs).
export const oracle = JSON.parse(readFileSync(join(repo, "tools", "openapi2baml", "tests", "fixtures", "naming-oracle.json"), "utf8"))

/** The oracle entry of the SDK operation `<group>.<method>`, with its operation ID as `id`. */
export function oracleEntry(svc, group, method) {
  // The SDK's operations, including the ones only its compatibility overlay declares.
  const entries = [
    ...Object.entries(oracle.operations[svc]).filter(([, e]) => e.in_sdk).map(([id, e]) => ({ ...e, id })),
    ...oracle.spec_vs_sdk_mismatches[svc].sdk_only.map((e) => ({ ...e, id: e.operation_id })),
  ]
  return entries.find((e) => e.group === group && e.method_name === method)
}

// ---------------------------------------------------------------------------
// Specs, for request body defaults (the importer's overlay rule: add what is absent).

function loadSpec(path, overlays) {
  const doc = JSON.parse(readFileSync(join(repo, path), "utf8"))
  const addAbsent = (target, update) => {
    for (const [key, value] of Object.entries(update)) {
      if (key in target && typeof target[key] === "object" && target[key] && typeof value === "object" && !Array.isArray(value)) addAbsent(target[key], value)
      else if (!(key in target)) target[key] = value
    }
  }
  for (const overlay of overlays) {
    for (const action of JSON.parse(readFileSync(join(repo, overlay), "utf8")).actions) {
      let node = doc
      for (const [, segment] of action.target.matchAll(/\['([^']*)'\]/g)) node = node[segment]
      addAbsent(node, action.update)
    }
  }
  return doc
}
export const specs = {
  platform: loadSpec("specs/platform/openapi.json", ["specs/platform/sdk-compatibility-overlay.json"]),
  manager: loadSpec("specs/manager/openapi.json", []),
}

export function deref(spec, schema) {
  while (schema && schema.$ref) {
    schema = schema.$ref.slice(2).split("/").reduce((node, part) => node[part.replaceAll("~1", "/").replaceAll("~0", "~")], spec)
  }
  return schema ?? {}
}

/** The schemas `key` may have inside `schema`: every allOf part and union member that lists it. */
function propertySchemas(spec, schema, key, out = []) {
  schema = deref(spec, schema)
  if (schema.properties?.[key]) out.push(schema.properties[key])
  for (const part of [...(schema.allOf ?? []), ...(schema.oneOf ?? []), ...(schema.anyOf ?? [])]) propertySchemas(spec, part, key, out)
  if (schema.items) propertySchemas(spec, schema.items, key, out)
  return out
}

/** The spec operation an expected request is for, its path-item parameters included. */
function specOperation(svc, expect) {
  const path = splitUrl(expect.path).path
  const paths = Object.entries(specs[svc].paths)
  // A literal path first: `/v1/deployments/import` before `/v1/deployments/{id}`.
  paths.sort(([a], [b]) => (a === path ? -1 : b === path ? 1 : 0))
  for (const [template, item] of paths) {
    const pattern = new RegExp("^" + template.replace(/[.*+?^$()|[\]\\]/g, "\\$&").replace(/\\?\{[^}]+\\?\}/g, "[^/]+") + "$")
    const op = item[expect.method.toLowerCase()]
    if (op && pattern.test(path)) return { ...op, parameters: [...(item.parameters ?? []), ...(op.parameters ?? [])] }
  }
  return null
}

/** The JSON schema of the response an exchange scripts. */
function responseSchema(svc, exchange) {
  const op = specOperation(svc, exchange.expect)
  const response = deref(specs[svc], op?.responses?.[String(exchange.respond?.status ?? 200)] ?? {})
  return response.content?.["application/json"]?.schema ?? null
}

/** The request body schema of the operation an expected request is for. */
function requestBodySchema(svc, expect) {
  const op = specOperation(svc, expect)
  return op?.requestBody ? deref(specs[svc], op.requestBody).content?.["application/json"]?.schema ?? null : null
}

/** Query parameter defaults of the operation an expected request is for: {name: default}. */
function queryDefaults(svc, expect) {
  const op = specOperation(svc, expect)
  const out = {}
  for (const raw of op?.parameters ?? []) {
    const param = deref(specs[svc], raw)
    const schema = deref(specs[svc], param.schema)
    if (param.in === "query" && "default" in schema) out[param.name] = schema.default
  }
  return out
}

function defaultsOf(spec, schema) {
  schema = deref(spec, schema)
  const out = "default" in schema ? [schema.default] : []
  for (const part of schema.allOf ?? []) out.push(...defaultsOf(spec, part))
  return out
}

// ---------------------------------------------------------------------------
// Comparison

const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/
const sameInstant = (a, b) => typeof a === "string" && typeof b === "string" && ISO.test(a) && ISO.test(b) && Date.parse(a) === Date.parse(b)

export const camel = (wire) => wire.split(/[^A-Za-z0-9]+/).filter(Boolean).map((w, i) => (i === 0 ? w[0].toLowerCase() + w.slice(1) : w[0].toUpperCase() + w.slice(1))).join("")

/**
 * Differences between `want` (what the BAML SDKs send or return) and `got`,
 * each `{path, kind, want, got}`. `kind` "equivalent:<why>" does not break
 * parity: a date-time written differently, and (requests, with `schema`) a
 * field the SDK fills with its spec default.
 */
function diff(want, got, path = "", ctx = null, schema = null) {
  if (isDeepStrictEqual(want, got)) return []
  if (sameInstant(want, got)) return [{ path, kind: "equivalent:date-time written differently", want, got }]
  const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v)
  if (isObj(want) && isObj(got)) {
    const out = []
    // A snake_case wire name the incumbent renamed (`custom_domains` → `customDomains`).
    got = { ...got }
    for (const key of Object.keys(want)) {
      const renamed = camel(key)
      if (renamed !== key && !(key in got) && renamed in got && !(renamed in want)) {
        out.push({ path: `${path}.${key}`, kind: "different: field name (the incumbent camelCases the wire name)", want: key, got: renamed })
        got[key] = got[renamed]
        delete got[renamed]
      }
    }
    for (const key of new Set([...Object.keys(want), ...Object.keys(got)])) {
      const at = `${path}.${key}`
      const w = want[key] ?? null
      const g = got[key] ?? null
      if (w === null && g === null) continue
      const subSchemas = ctx && schema ? propertySchemas(ctx, schema, key) : []
      if (w === null && ctx && subSchemas.some((s) => defaultsOf(ctx, s).some((d) => isDeepStrictEqual(d, g)))) {
        out.push({ path: at, kind: "equivalent:spec default filled in", want: w, got: g })
        continue
      }
      out.push(...diff(w, g, at, ctx, subSchemas[0] ?? null))
    }
    return out
  }
  if (Array.isArray(want) && Array.isArray(got) && want.length === got.length) {
    const items = ctx && schema ? deref(ctx, schema).items ?? null : null
    return want.flatMap((w, i) => diff(w, got[i], `${path}[${i}]`, ctx, items))
  }
  return [{ path: path || "(value)", kind: got === null ? "different: left out" : want === null ? "different: added" : "different", want, got }]
}

export const normalize = (v) => (v === undefined ? null : JSON.parse(JSON.stringify(v)))
const dropNulls = (v) => {
  if (Array.isArray(v)) return v.map(dropNulls)
  if (v !== null && typeof v === "object") return Object.fromEntries(Object.entries(v).filter(([, x]) => x !== null && x !== undefined).map(([k, x]) => [k, dropNulls(x)]))
  return v
}

/** Path and query as an ordered list of decoded pairs. */
function splitUrl(url) {
  // Appended, not resolved: a path that starts with `//` is not a host.
  const u = new URL("http://stub" + url)
  return { path: decodeURIComponent(u.pathname), query: [...u.searchParams.entries()] }
}

function requestDifferences(expect, captured, svc, bodySchema) {
  const out = []
  if (!captured) return [{ path: "request", kind: "different", want: `${expect.method} ${expect.path}`, got: "no request" }]
  if (expect.method !== captured.method) out.push({ path: "method", kind: "different", want: expect.method, got: captured.method })
  const want = splitUrl(expect.path)
  const got = splitUrl(captured.path)
  if (want.path !== got.path) out.push({ path: "path", kind: "different", want: want.path, got: got.path })
  // A date-time written differently is the same value (`…00Z`, `…00.000Z`).
  const instant = (q) => q.map(([n, v]) => [n, ISO.test(v) ? new Date(Date.parse(v)).toISOString() : v])
  if (!isDeepStrictEqual(want.query, got.query) && isDeepStrictEqual(instant(want.query), instant(got.query))) {
    out.push({ path: "query", kind: "equivalent:date-time written differently", want: want.query.map((p) => p.join("=")).join("&"), got: got.query.map((p) => p.join("=")).join("&") })
    got.query = want.query
  }
  if (!isDeepStrictEqual(want.query, got.query)) {
    const pairs = (q) => instant(q).map((p) => p.join("=")).sort()
    // Parameters the incumbent adds with their spec default (`limit=20`).
    const defaults = queryDefaults(svc, expect)
    const extra = got.query.filter(([name, value]) => !want.query.some(([n]) => n === name))
    const filled = extra.filter(([name, value]) => name in defaults && String(defaults[name]) === value)
    const rest = got.query.filter((pair) => !filled.includes(pair))
    const text = (q) => q.map((p) => p.join("=")).join("&")
    if (filled.length > 0) out.push({ path: "query", kind: "equivalent:spec default filled in (query)", want: null, got: text(filled) })
    if (!isDeepStrictEqual(instant(want.query), instant(rest))) {
      const kind = isDeepStrictEqual(pairs(want.query), pairs(rest)) ? "equivalent:query parameters in another order" : "different"
      out.push({ path: "query", kind, want: text(want.query), got: text(rest) })
    }
  }
  // Headers the scenario names (it names authorization and content-type for operations).
  const baseType = (v) => v?.split(";")[0].trim() ?? null
  for (const [name, value] of Object.entries(expect.headers ?? {})) {
    if (name === "accept") continue
    const g = captured.headers[name] ?? null
    if (value !== g && !(name === "content-type" && baseType(value) === baseType(g))) out.push({ path: `header ${name}`, kind: "different", want: value, got: g })
  }
  for (const name of expect.headers_absent ?? []) {
    const g = captured.headers[name]
    if (g === undefined) continue
    const kind = name === "content-type" && captured.body_text === "" ? "equivalent:content-type sent without a body" : "different"
    out.push({ path: `header ${name}`, kind, want: "(absent)", got: g })
  }
  if ("json" in expect) {
    if (captured.json === undefined) out.push({ path: "body", kind: "different", want: expect.json, got: captured.body_text })
    else out.push(...diff(expect.json, captured.json, "body", specs[svc], bodySchema).map((d) => ({ ...d, path: d.path || "body" })))
  } else if (expect.body_empty && captured.body_text !== "") {
    out.push({ path: "body", kind: "different", want: "(empty)", got: captured.body_text })
  }
  return out
}

function errorDifferences(wantError, gotError) {
  if (!gotError) return [{ path: "outcome", kind: "different", want: wantError, got: "success" }]
  const out = []
  for (const key of ["code", "retryable", "http_status"]) {
    const g = gotError[key] ?? null
    if (key === "retryable" && g === null) continue // not an API error body: no retryable flag
    if ((wantError[key] ?? null) !== g) out.push({ path: `error.${key}`, kind: "different", want: wantError[key] ?? null, got: g })
  }
  // Context, when the scenario lists one: what the incumbent reports must
  // match; `path` is the BAML SDKs' own.
  for (const [key, value] of Object.entries(wantError.context ? gotError.context ?? {} : {})) {
    if (value !== null && wantError.context?.[key] !== value) out.push({ path: `error.context.${key}`, kind: "different", want: wantError.context?.[key] ?? null, got: value })
  }
  return out
}

/**
 * Judges one scenario: `captured` holds the requests the incumbent sent
 * (`{method, path, headers, body_text, json}`), `outcome` is `{ok}` (the
 * decoded result as JSON) or `{error}` (reduced to `{code, retryable,
 * http_status}` and the context the incumbent reports).
 */
export function judge(scenario, svc, captured, outcome) {
  // Requests: each one the scenario scripts, against what the SDK sent.
  const request = (scenario.exchanges ?? []).flatMap((exchange, i) =>
    requestDifferences(exchange.expect, captured[i], svc, requestBodySchema(svc, exchange.expect)).map((d) => ({ ...d, exchange: i + 1 })))
  if (captured.length > (scenario.exchanges ?? []).length) {
    request.push({ path: "requests", kind: "different", want: (scenario.exchanges ?? []).length, got: captured.length })
  }
  const result = scenario.expect_error
    ? errorDifferences(scenario.expect_error, outcome.error)
    : outcome.error
      ? [{ path: "outcome", kind: "different", want: "success", got: outcome.error }]
      : diff(dropNulls(normalize(scenario.expect_result?.ok ?? null)), dropNulls(outcome.ok), "result", specs[svc],
        scenario.exchanges?.length ? responseSchema(svc, scenario.exchanges.at(-1)) : null)
  return { request, result }
}

// ---------------------------------------------------------------------------
// The stub (conformance/stub-server)

/** Starts the stub server: `{base, post(path, body), stop()}`. */
export async function startStub() {
  const stub = spawn(process.execPath, [join(repo, "conformance", "stub-server", "server.mjs")], { stdio: ["ignore", "pipe", "inherit"] })
  const base = await new Promise((ok, fail) => {
    createInterface({ input: stub.stdout }).once("line", (line) => (line.startsWith("READY ") ? ok(line.slice(6)) : fail(new Error(line))))
  })
  const post = async (path, body) => (await fetch(base + path, { method: "POST", body: JSON.stringify(body) })).json()
  return { base, post, stop: () => stub.kill() }
}

// ---------------------------------------------------------------------------
// Output: conformance/baseline/<adapter>/

/**
 * Writes one file per scenario and SUMMARY.md to `outDir`. Baselines are
 * checked in, so the stub's address, fixture secrets and instants derived
 * from the run's clock are written back as templates: two runs produce
 * identical files.
 */
export class Baseline {
  /** `bases`: the stub's address, and any address standing in for it (a proxy). */
  constructor(outDir, ...bases) {
    this.outDir = outDir
    // Longest first: one address can be a prefix of another (`…:6513`, `…:65133`).
    this.bases = bases.sort((a, b) => b.length - a.length)
    this.startedAt = Date.now()
    this.secrets = JSON.parse(readFileSync(join(repo, "conformance", "secrets.json"), "utf8"))
    this.results = []
    rmSync(outDir, { recursive: true, force: true })
    mkdirSync(outDir, { recursive: true })
  }

  templatize(value) {
    if (typeof value === "string") {
      let out = value
      for (const base of this.bases) out = out.replaceAll(base, "${stub.base}")
      for (const [name, secret] of Object.entries(this.secrets)) out = out.replaceAll(secret, `\${secrets.${name}}`)
      return out.replace(/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z/g, (iso) =>
        Math.abs(Date.parse(iso) - this.startedAt) < 15 * 60_000 ? "${now…}" : iso)
    }
    if (Array.isArray(value)) return value.map((v) => this.templatize(v))
    if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, this.templatize(v)]))
    return value
  }

  skip(id, reason) {
    this.results.push({ id, status: "skipped", reason })
    writeFileSync(join(this.outDir, `${id}.json`), JSON.stringify({ skipped: reason }, null, 2) + "\n")
  }

  /** Records a judged scenario (`judge`), and prints one line about it. */
  record(id, outcome, { request, result }) {
    const differences = [...request, ...result]
    const same = differences.every((d) => d.kind.startsWith("equivalent:"))
    const record = this.templatize({ outcome, request_parity: request.every((d) => d.kind.startsWith("equivalent:")), outcome_parity: result.every((d) => d.kind.startsWith("equivalent:")), differences })
    writeFileSync(join(this.outDir, `${id}.json`), JSON.stringify(record, null, 2) + "\n")
    this.results.push({ id, status: same ? "same" : "different", request: record.request_parity, outcome: record.outcome_parity, differences: record.differences })
    console.log(`${same ? "SAME" : "DIFF"} ${id}${same ? "" : `: ${differences.filter((d) => !d.kind.startsWith("equivalent:")).map((d) => `${d.path} ${JSON.stringify(d.want)} -> ${JSON.stringify(d.got)}`).join("; ").slice(0, 300)}`}`)
  }

  /** SUMMARY.md: `header` is its title and introduction, as lines. */
  summary(header, skippedLabel) {
    const results = this.results
    const count = (pred) => results.filter(pred).length
    const run = results.filter((r) => r.status !== "skipped")
    // Each difference by kind: what differs, and for an error which error class.
    function kindOf(d) {
      if (d.kind.startsWith("different: left out") || d.kind.startsWith("different: added")) {
        return `${d.kind} (${d.path.startsWith("result") ? "result" : d.path.startsWith("body") ? "request body" : d.path} field)`
      }
      if (d.kind !== "different") return d.kind
      if (d.path === "request") return "different: no request sent"
      if (d.path === "error.code") return `different: error code (BAML SDKs ${typeof d.want === "string" && d.want.startsWith("(") ? d.want : "an AlienError code"}, incumbent ${d.got})`
      if (d.path === "outcome") return `different: ${typeof d.got === "object" ? `error ${d.got.code} instead of a result` : "a result instead of an error"}`
      return `different: ${d.path.replace(/\[\d+\]/g, "[]").replace(/^(result|body)\..*/, "$1 field")}`
    }
    const kinds = new Map()
    for (const r of run) {
      for (const d of r.differences) kinds.set(kindOf(d), (kinds.get(kindOf(d)) ?? 0) + 1)
    }
    const cell = (text) => String(text).replaceAll("|", "\\|").replaceAll("\n", " ")
    const lines = [
      ...header,
      "",
      `| | Scenarios |`,
      `|---|---|`,
      `| Run | ${run.length} |`,
      `| Same request and outcome | ${count((r) => r.status === "same")} |`,
      `| Same request | ${count((r) => r.status !== "skipped" && r.request)} |`,
      `| Same outcome | ${count((r) => r.status !== "skipped" && r.outcome)} |`,
      `| ${skippedLabel} | ${count((r) => r.status === "skipped")} |`,
      "",
      "## Differences by kind",
      "",
      "| Kind | Occurrences |",
      "|---|---|",
      ...[...kinds.entries()].sort((a, b) => b[1] - a[1]).map(([k, n]) => `| ${cell(k)} | ${n} |`),
      "",
      "## Scenarios",
      "",
      "| Scenario | Request | Outcome | Differences (not equivalent) |",
      "|---|---|---|---|",
      ...results.map((r) => r.status === "skipped"
        ? `| ${r.id} | — | — | skipped: ${cell(r.reason)} |`
        : `| ${r.id} | ${r.request ? "same" : "**different**"} | ${r.outcome ? "same" : "**different**"} | ${cell(r.differences.filter((d) => !d.kind.startsWith("equivalent:")).map((d) => `${d.path}: ${JSON.stringify(d.want)} → ${JSON.stringify(d.got)}`).join("; ").slice(0, 300))} |`),
      "",
    ]
    writeFileSync(join(this.outDir, "SUMMARY.md"), this.templatize(lines.join("\n")))
    console.log(`${count((r) => r.status === "same")} of ${run.length} scenarios behave the same in the incumbents (${count((r) => r.status === "skipped")} skipped)`)
  }
}
