#!/usr/bin/env node
// Scripted HTTP stub for the conformance scenarios (conformance/README.md).
//
// Modelled on Alien's packages/commands/tests/helpers/stub-server.ts, but run
// as its own process so every language harness can drive the same server.
//
//   node server.mjs [--port N] [--secrets path/to/secrets.json]
//
// Prints `READY http://127.0.0.1:<port>` once listening. Control endpoints live
// under `/__`; every other request must match the next scripted exchange of the
// loaded scenario, or it is recorded as a failure and answered with 599.
//
// Zero dependencies on purpose: Node >= 20 only.

import { createServer } from "node:http"
import { readFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { isDeepStrictEqual } from "node:util"
import { normalize, compareOutcome } from "./normalize.mjs"

const here = dirname(fileURLToPath(import.meta.url))

function parseArgs(argv) {
  const args = { port: 0, secrets: join(here, "..", "secrets.json") }
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--port") args.port = Number(argv[++i])
    else if (argv[i] === "--secrets") args.secrets = argv[++i]
    else throw new Error(`unknown argument: ${argv[i]}`)
  }
  return args
}

const args = parseArgs(process.argv.slice(2))
const secrets = JSON.parse(readFileSync(args.secrets, "utf8"))
const started = process.hrtime.bigint()
const elapsedMs = () => Number(process.hrtime.bigint() - started) / 1e6

let base = ""
let state = emptyState()

function emptyState() {
  return { scenario: null, exchanges: [], cursor: 0, requests: [], failures: [], outcome: null }
}

// `${secrets.x}`, `${stub.base}`, `${now+300s}` / `${now-10s}` (ISO-8601, UTC, ms precision),
// and `${$}` for a literal `$` (`${$}{x}` is the text `${x}`).
// `now` is fixed per loaded scenario, so every occurrence names the same instant.
let loadedAt = Date.now()

function resolveString(text) {
  return text.replace(/\$\{([^}]+)\}/g, (whole, expr) => {
    if (expr === "$") return "$"
    if (expr === "stub.base") return base
    if (expr.startsWith("secrets.")) {
      const key = expr.slice("secrets.".length)
      if (!(key in secrets)) throw new Error(`unknown secret in template: ${whole}`)
      return secrets[key]
    }
    const now = /^now([+-]\d+)s$/.exec(expr)
    if (now) return new Date(loadedAt + Number(now[1]) * 1000).toISOString()
    if (expr === "now") return new Date(loadedAt).toISOString()
    throw new Error(`unknown template: ${whole}`)
  })
}

function resolve(value) {
  if (typeof value === "string") return resolveString(value)
  if (Array.isArray(value)) return value.map(resolve)
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, resolve(v)]))
  }
  return value
}

function readBody(req) {
  return new Promise((ok, fail) => {
    const chunks = []
    req.on("data", c => chunks.push(c))
    req.on("end", () => ok(Buffer.concat(chunks)))
    req.on("error", fail)
  })
}

function sendJson(res, status, value) {
  const body = JSON.stringify(value)
  res.writeHead(status, { "content-type": "application/json", "content-length": Buffer.byteLength(body) })
  res.end(body)
}

function lowerHeaders(headers) {
  return Object.fromEntries(Object.entries(headers).map(([k, v]) => [k.toLowerCase(), v]))
}

// Returns a list of mismatch descriptions; empty means the request matches.
function matchRequest(expect, captured) {
  const problems = []
  if (expect.method && expect.method !== captured.method) {
    problems.push(`method: expected ${expect.method}, got ${captured.method}`)
  }
  if (expect.path !== undefined && expect.path !== captured.path) {
    problems.push(`path: expected ${expect.path}, got ${captured.path}`)
  }
  for (const [name, want] of Object.entries(expect.headers ?? {})) {
    const got = captured.headers[name.toLowerCase()]
    if (got !== want) problems.push(`header ${name}: expected ${JSON.stringify(want)}, got ${JSON.stringify(got)}`)
  }
  for (const name of expect.headers_absent ?? []) {
    if (captured.headers[name.toLowerCase()] !== undefined) problems.push(`header ${name}: expected absent`)
  }
  if ("json" in expect) {
    if (captured.json === undefined) problems.push(`body: expected JSON ${JSON.stringify(expect.json)}, got ${JSON.stringify(captured.body_text)}`)
    else if (!isDeepStrictEqual(captured.json, expect.json)) {
      problems.push(`json: expected ${JSON.stringify(normalize(expect.json))}, got ${JSON.stringify(normalize(captured.json))}`)
    }
  }
  if (expect.body_empty && captured.body_text !== "") problems.push(`body: expected empty, got ${JSON.stringify(captured.body_text)}`)
  return problems
}

function respond(res, spec, req) {
  const delay = spec.delay_ms ?? 0
  setTimeout(() => {
    if (spec.drop) {
      // Simulate a network failure: no status line, connection reset.
      req.socket.destroy()
      return
    }
    const headers = { ...(spec.headers ?? {}) }
    let body = ""
    if ("json" in spec) {
      body = JSON.stringify(spec.json)
      headers["content-type"] ??= "application/json"
    } else if ("text" in spec) {
      body = spec.text
      headers["content-type"] ??= "text/plain"
    }
    headers["content-length"] = Buffer.byteLength(body)
    res.writeHead(spec.status ?? 200, headers)
    res.end(body)
  }, delay)
}

// Evaluates the scenario's `assert` list against the request log.
function evaluateAsserts(scenario, requests) {
  const problems = []
  for (const rule of scenario.assert ?? []) {
    const [kind, arg] = rule.split(/:(.*)/s)
    if (kind === "api_key_never_sent_to") {
      for (const r of requests.filter(r => r.path.startsWith(arg))) {
        if (JSON.stringify(r).includes(secrets.apiKey)) problems.push(`${rule}: API key reached ${r.method} ${r.path}`)
      }
    } else if (kind === "no_requests") {
      if (requests.length > 0) problems.push(`${rule}: expected no requests, got ${requests.length}`)
    } else {
      problems.push(`unknown assert rule: ${rule}`)
    }
  }
  return problems
}

const server = createServer(async (req, res) => {
  const bodyBuf = await readBody(req)
  const path = req.url ?? ""

  if (path.startsWith("/__")) {
    try {
      if (req.method === "POST" && path === "/__load") {
        // Templates are resolved once, so `${now+300s}` is the same instant in
        // a scripted response and in the expected result.
        loadedAt = Date.now()
        const scenario = resolve(JSON.parse(bodyBuf.toString("utf8")).scenario)
        state = emptyState()
        state.scenario = scenario
        state.exchanges = scenario.exchanges ?? []
        return sendJson(res, 200, { call: scenario.call, base, scenario })
      }
      if (req.method === "GET" && path === "/__log") {
        return sendJson(res, 200, {
          requests: state.requests,
          failures: state.failures,
          consumed: state.cursor,
          total: state.exchanges.length,
        })
      }
      if (req.method === "POST" && path === "/__result") {
        // The harness reports the facade's outcome; the stub compares it with
        // the scenario, so every language uses one comparison implementation.
        const outcome = JSON.parse(bodyBuf.toString("utf8"))
        const problems = [...state.failures]
        if (state.cursor !== state.exchanges.length) {
          problems.push(`exchanges: ${state.exchanges.length - state.cursor} scripted exchange(s) never requested`)
        }
        problems.push(...evaluateAsserts(state.scenario, state.requests))
        problems.push(...compareOutcome(state.scenario, outcome))
        state.outcome = outcome
        return sendJson(res, 200, { ok: problems.length === 0, problems, normalized: normalize(outcome) })
      }
      return sendJson(res, 404, { error: `unknown control endpoint ${req.method} ${path}` })
    } catch (error) {
      return sendJson(res, 400, { error: String(error?.stack ?? error) })
    }
  }

  const bodyText = bodyBuf.toString("utf8")
  let json
  try {
    json = bodyText === "" ? undefined : JSON.parse(bodyText)
  } catch {
    json = undefined
  }
  const captured = {
    t_ms: elapsedMs(),
    method: req.method ?? "",
    path,
    headers: lowerHeaders(req.headers),
    body_text: bodyText,
    json,
  }
  state.requests.push(captured)

  const exchange = state.exchanges[state.cursor]
  if (exchange === undefined) {
    state.failures.push(`unexpected request #${state.requests.length}: ${captured.method} ${captured.path} (no exchanges left)`)
    return sendJson(res, 599, { stub_error: "unexpected request" })
  }
  const problems = matchRequest(exchange.expect ?? {}, captured)
  if (problems.length > 0) {
    state.failures.push(`exchange #${state.cursor + 1} (${captured.method} ${captured.path}): ${problems.join("; ")}`)
    return sendJson(res, 599, { stub_error: "request did not match the scripted exchange", problems })
  }
  state.cursor++
  respond(res, exchange.respond ?? {}, req)
})

server.listen(args.port, "127.0.0.1", () => {
  base = `http://127.0.0.1:${server.address().port}`
  process.stdout.write(`READY ${base}\n`)
})

for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => server.close(() => process.exit(0)))
