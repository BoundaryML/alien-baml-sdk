// Facade behavior the shared conformance scenarios do not cover: client
// options and their environment defaults, Manager authentication, optional
// arguments, request bodies as plain objects and as classes, error fields, and
// what printing a client shows. Every request goes to a server on 127.0.0.1.
import { readFileSync } from "node:fs"
import { createServer, type Server } from "node:http"
import type { AddressInfo } from "node:net"
import { inspect } from "node:util"
import { BamlAbortError } from "@boundaryml/baml-bridge"
import { afterAll, afterEach, beforeAll, describe, expect, expectTypeOf, test, vi } from "vitest"
import { Alien, AlienError, AlienManager, type platform, shutdown } from "../src/index.js"
import * as sdk from "../baml_sdk/index.js"

const KEY = "alien_test_key_6f1c"
const TOKEN = "manager_test_token_93ad"

interface Seen {
  method: string
  url: string
  authorization: string | undefined
  body: string
}

const seen: Seen[] = []
let response = { status: 200, body: "{}", delayMs: 0 }
let server: Server
let serverURL: string

function respond(status: number, body: unknown, delayMs = 0): void {
  response = { status, body: JSON.stringify(body), delayMs }
}

function last(): Seen {
  const request = seen.at(-1)
  if (!request) throw new Error("no request reached the server")
  return request
}

beforeAll(async () => {
  server = createServer((req, res) => {
    let body = ""
    req.on("data", (chunk: Buffer) => (body += chunk))
    req.on("end", () => {
      seen.push({ method: req.method ?? "", url: req.url ?? "", authorization: req.headers.authorization, body })
      const { status, body: reply, delayMs } = response
      setTimeout(() => res.writeHead(status, { "content-type": "application/json" }).end(reply), delayMs)
    })
  })
  await new Promise<void>((ready) => server.listen(0, "127.0.0.1", ready))
  serverURL = `http://127.0.0.1:${(server.address() as AddressInfo).port}`
})

afterEach(() => {
  vi.unstubAllEnvs()
})

afterAll(async () => {
  server.close()
  // Flush telemetry: Vitest ends workers without Node's beforeExit.
  await shutdown()
})

const emptyPage = { items: [], nextCursor: null }

describe("Alien options", () => {
  test("apiKey, serverURL and workspace", async () => {
    respond(200, emptyPage)
    const alien = new Alien({ apiKey: KEY, serverURL, workspace: "acme" })
    const page = await alien.deployments.list({ project: "web", status: ["running", "failed"], limit: 10 })
    expect(page.items).toEqual([])
    expect(last()).toEqual({
      method: "GET",
      url: "/v1/deployments?workspace=acme&project=web&status=running&status=failed&limit=10",
      authorization: `Bearer ${KEY}`,
      body: "",
    })
  })

  test("apiKey and workspace default to ALIEN_API_KEY and ALIEN_WORKSPACE", async () => {
    vi.stubEnv("ALIEN_API_KEY", "key_from_env")
    vi.stubEnv("ALIEN_WORKSPACE", "ws_from_env")
    respond(200, emptyPage)
    await new Alien({ serverURL }).deployments.list()
    expect(last()).toMatchObject({ url: "/v1/deployments?workspace=ws_from_env", authorization: "Bearer key_from_env" })
    await new Alien({ apiKey: KEY, serverURL, workspace: "acme" }).deployments.list()
    expect(last()).toMatchObject({ url: "/v1/deployments?workspace=acme", authorization: `Bearer ${KEY}` })
  })

  test("an API key is required", () => {
    const noKey = { code: "INVALID_REQUEST", message: "no API key: pass apiKey or set ALIEN_API_KEY", retryable: false, statusCode: null }
    vi.stubEnv("ALIEN_API_KEY", undefined)
    expect(() => new Alien({ serverURL })).toThrow(AlienError)
    expect(() => new Alien({ serverURL })).toThrow(expect.objectContaining(noKey))
    expect(() => new Alien({ apiKey: "", serverURL })).toThrow(expect.objectContaining(noKey))
    vi.stubEnv("ALIEN_API_KEY", "")
    expect(() => new Alien({ serverURL })).toThrow(expect.objectContaining(noKey))
  })

  test("an apiKey function is called before every request", async () => {
    let calls = 0
    const alien = new Alien({ apiKey: async () => `key_${++calls}`, serverURL })
    respond(200, [])
    await alien.managers.list()
    expect(last().authorization).toBe("Bearer key_1")
    await alien.managers.list()
    expect(last().authorization).toBe("Bearer key_2")
    expect(calls).toBe(2)
  })

  test("timeoutMs is an integer number of milliseconds", () => {
    expect(() => new Alien({ apiKey: KEY, serverURL, timeoutMs: 1.5 })).toThrow(
      new TypeError("timeoutMs must be an integer number of milliseconds, got 1.5"),
    )
    expect(() => new AlienManager({ serverURL, timeoutMs: Number.NaN })).toThrow(TypeError)
    expect(() => new Alien({ apiKey: KEY, serverURL, timeoutMs: 0 })).not.toThrow()
  })

  test("a per-call timeout shorter than timeoutMs ends the call first, with BamlAbortError", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL, timeoutMs: 5_000 })
    respond(200, {}, 1_000)
    await expect(alien.managers.get("mgr_x", { $baml: { timeoutMs: 100 } })).rejects.toBeInstanceOf(BamlAbortError)
  })

  test("an apiKey function that returns no key fails the call before any request", async () => {
    const before = seen.length
    const alien = new Alien({ apiKey: async () => "", serverURL })
    await expect(alien.managers.list()).rejects.toThrow(
      expect.objectContaining({ code: "INVALID_REQUEST", message: "no API key: the apiKey function returned none" }),
    )
    expect(seen.length).toBe(before)
  })
})

describe("AlienManager", () => {
  const created = { commandId: "cmd_1", state: "PENDING", storageUploadRequest: null, inlineAllowedUpTo: 100, next: "poll" }

  test("token is sent on every operation; a union body can be a plain object", async () => {
    const manager = new AlienManager({ serverURL, token: TOKEN })
    respond(200, created)
    const result = await manager.commands.createCommand({
      command: "health-check",
      deploymentId: "dep_1",
      params: { mode: "inline", inlineBase64: "e30=" },
    })
    expect(result.commandId).toBe("cmd_1")
    expect(last()).toMatchObject({ method: "POST", url: "/v1/commands", authorization: `Bearer ${TOKEN}` })
    expect(JSON.parse(last().body)).toEqual({ command: "health-check", deploymentId: "dep_1", params: { mode: "inline", inlineBase64: "e30=" } })
  })

  test("without a token no Authorization header is sent", async () => {
    respond(200, created)
    const body = { command: "c", deploymentId: "d", params: { mode: "inline" as const, inlineBase64: "" } }
    await new AlienManager({ serverURL }).commands.createCommand(body)
    expect(last().authorization).toBeUndefined()
    await new AlienManager({ serverURL, token: "" }).commands.createCommand(body)
    expect(last().authorization).toBeUndefined()
  })

  test("serverURL is required", () => {
    expect(() => new AlienManager({ baseUrl: serverURL } as never)).toThrow(new TypeError("AlienManager needs `serverURL`"))
  })
})

describe("arguments and request bodies", () => {
  test("an optional body goes in the options object; leaving it out sends no body", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL })
    respond(200, { token: "t", deploymentLink: "https://example.com/d" })
    await alien.deploymentGroups.createToken("dg_1", { body: { description: "CI" } })
    expect(last()).toMatchObject({ method: "POST", url: "/v1/deployment-groups/dg_1/tokens", body: '{"description":"CI"}' })
    await alien.deploymentGroups.createToken("dg_1")
    expect(last().body).toBe("")
  })

  // The generated classes are not exported, but responses are instances of
  // them and can be sent back as request bodies.
  test("a union body can be a plain object or a generated class", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL })
    respond(200, { managerUrl: "https://manager.example.com", token: "t", expiresAt: "2026-01-01T00:00:00Z" })
    await alien.commands.bootstrap({ deploymentId: "dep_1", role: "sender" })
    const plain = last().body
    await alien.commands.bootstrap(new sdk.platform.CommandBootstrapRequestSender({ deploymentId: "dep_1", role: "sender" }))
    expect(JSON.parse(last().body)).toEqual({ deploymentId: "dep_1", role: "sender" })
    expect(last().body).toBe(plain)
  })

  test("a class body leaves unset nullable fields out", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL })
    respond(400, { code: "BAD_REQUEST", message: "stop here", retryable: false, internal: false })
    const request = new sdk.platform.CreateSetupLinkRequest({ externalId: "cust-1", name: "Customer", project: "web" })
    await expect(alien.setupLinks.create(request)).rejects.toBeInstanceOf(AlienError)
    expect(JSON.parse(last().body)).toEqual({ externalId: "cust-1", name: "Customer", project: "web" })
  })
})

describe("union members this SDK does not know", () => {
  const event = (id: string, data: object) => ({
    id, data, state: "success", projectId: "prj_1", createdAt: "2026-10-01T00:00:00Z", workspaceId: "ws_1",
  })

  // A plain switch on the tag, checked exhaustively by tsc: the member this SDK
  // does not know is `case "UNKNOWN"` (docs/api-shape.md, "Unions").
  function describeSubject(subject: platform.Subject): string {
    switch (subject.kind) {
      case "user":
        return `user ${subject.email}`
      case "serviceAccount":
        return `service account, scope ${subject.scope.type}`
      case "UNKNOWN":
        return `unknown ${String(subject.raw.kind)}`
      default: {
        const unhandled: never = subject
        return unhandled
      }
    }
  }

  test("an event type newer than the SDK has the tag UNKNOWN and keeps the event in raw", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL })
    const unknown = { type: "SomethingNew", deploymentId: "dep_1", detail: { attempt: 2 } }
    respond(200, {
      items: [event("event_1", { type: "DeploymentCheckRequested", deploymentId: "dep_1" }), event("event_2", unknown)],
      nextCursor: null,
    })
    const got: unknown[][] = []
    for (const e of (await alien.events.list()).items) {
      switch (e.data.type) {
        case "DeploymentCheckRequested":
          expectTypeOf(e.data).toEqualTypeOf<platform.DeploymentCheckRequestedEvent>()
          got.push([e.data.type, e.data.deploymentId])
          break
        case "UNKNOWN":
          expectTypeOf(e.data).toEqualTypeOf<platform.EventDataOther>()
          got.push([e.data.type, e.data.raw])
          break
      }
    }
    expect(got).toEqual([
      ["DeploymentCheckRequested", "dep_1"],
      ["UNKNOWN", unknown],
    ])
  })

  test("a switch on the tag handles them like any other member, also nested and as a whole response", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL })
    respond(200, { kind: "robot", id: "r_1", serial: 7 })
    expect(describeSubject(await alien.auth.whoami())).toBe("unknown robot")
    respond(200, { kind: "serviceAccount", id: "sa_1", workspaceId: "ws_1", role: "admin", scope: { type: "galaxy", id: "g_1" } })
    expect(describeSubject(await alien.auth.whoami())).toBe("service account, scope UNKNOWN")
    respond(200, { kind: "user", id: "u_1", email: "a@example.com", workspaceId: "ws_1", role: "admin" })
    expect(describeSubject(await alien.auth.whoami())).toBe("user a@example.com")
  })

  test("a known tag that does not decode still fails", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL })
    respond(200, { kind: "user", id: "u_1" })
    await expect(alien.auth.whoami()).rejects.toMatchObject({ code: "MALFORMED_RESPONSE" })
  })

  test("a union tagged by two fields has one catch-all: a known type with a new service is UNKNOWN", async () => {
    // A deployment with an S3 binding and a `storage` binding whose `service`, `minio`, this SDK does not know.
    const scenario = new URL(
      "../../../conformance/scenarios/ops.platform.deployments.get.external_binding_new_service.json",
      import.meta.url,
    )
    respond(200, JSON.parse(readFileSync(scenario, "utf8")).exchanges[0].respond.json)
    const deployment = await new Alien({ apiKey: KEY, serverURL }).deployments.get("dep_1")
    const got: unknown[] = []
    for (const b of Object.values(deployment.stackSettings.externalBindings ?? {})) {
      if (b.type === "UNKNOWN") got.push(b.raw.service)
      else if (b.type === "storage" && b.service === "s3") got.push(b.bucketName)
    }
    expect(got).toEqual(["files", "minio"])
  })

  test("a switch without the UNKNOWN case is not exhaustive (checked by tsc -p test)", () => {
    const subject = { kind: "UNKNOWN", raw: { kind: "robot" } } as platform.Subject
    switch (subject.kind) {
      case "user":
      case "serviceAccount":
        break
      default: {
        // @ts-expect-error: SubjectOther, the member tagged "UNKNOWN", is not handled
        const unhandled: never = subject
        expect(unhandled).toBe(subject)
      }
    }
  })
})

describe("errors", () => {
  test("an API error keeps Alien's field names", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL })
    respond(404, {
      code: "NOT_FOUND",
      message: "Manager not found",
      retryable: false,
      internal: false,
      hint: "Check the manager ID",
      requestId: "req_1",
      context: { id: "mgr_x" },
    })
    const error = await alien.managers.get("mgr_x").catch((e: unknown) => e)
    expect(error).toBeInstanceOf(AlienError)
    const e = error as AlienError
    expect([e.name, e.code, e.message, e.retryable, e.statusCode]).toEqual(["AlienError", "NOT_FOUND", "Manager not found", false, 404])
    expect([e.hint, e.requestId]).toEqual(["Check the manager ID", "req_1"])
    expect(e.context).toEqual({ path: "/v1/managers/mgr_x", requestId: "req_1", hint: "Check the manager ID", details: '{"id":"mgr_x"}' })
    // A plain object, not a null-prototype map.
    expect(Object.getPrototypeOf(e.context)).toBe(Object.prototype)
    expect(e.context?.hasOwnProperty("path")).toBe(true)
  })

  test("an error without a response has no statusCode, hint or requestId", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL: "http://127.0.0.1:9" })
    const e = (await alien.managers.get("mgr_x").catch((e: unknown) => e)) as AlienError
    expect([e.code, e.retryable, e.statusCode, e.hint, e.requestId]).toEqual(["COMMUNICATION_ERROR", true, null, null, null])
  })

  test("a cancelled call throws the runtime's BamlAbortError, not AlienError", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL })
    const signal = AbortSignal.abort()
    await expect(alien.managers.get("mgr_x", { $baml: { signal } })).rejects.toBeInstanceOf(BamlAbortError)
  })
})

describe("printing", () => {
  test("clients and errors do not show the API key or token", async () => {
    const alien = new Alien({ apiKey: KEY, serverURL })
    const manager = new AlienManager({ serverURL, token: TOKEN })
    respond(500, { code: "INTERNAL", message: "boom", retryable: true, internal: false })
    const error = await alien.managers.get("mgr_x").catch((e: unknown) => e)
    const managerError = await manager.commands.getCommandStatus("cmd_1").catch((e: unknown) => e)
    const printed = [alien, alien.setup, alien.deployments, manager, error, managerError].flatMap((value) => [
      inspect(value, { depth: null, showHidden: true }),
      String(value),
      JSON.stringify(value) ?? "",
    ])
    for (const text of printed) {
      expect(text).not.toContain(KEY)
      expect(text).not.toContain(TOKEN)
    }
  })
})

describe("every operation of Alien's TypeScript SDKs", () => {
  // Extracted from the SDKs' sources (tools/openapi2baml/tests/naming_oracle.rs).
  const oracle = JSON.parse(
    readFileSync(new URL("../../../tools/openapi2baml/tests/fixtures/naming-oracle.json", import.meta.url), "utf8"),
  )
  type Entry = { in_sdk: boolean; group: string | null; method_name: string }
  const clients: Record<string, () => Record<string, any>> = {
    platform: () => new Alien({ apiKey: KEY, serverURL }) as any,
    manager: () => new AlienManager({ serverURL }) as any,
  }
  for (const [service, client] of Object.entries(clients)) {
    test(`${service}: is a method with the same group and name`, () => {
      const entries = Object.values(oracle.operations[service] as Record<string, Entry>).filter((e) => e.in_sdk)
      expect(entries.length).toBeGreaterThan(50)
      const missing = entries.filter((e) => {
        const target = e.group === null ? client() : client()[e.group]
        return typeof target?.[e.method_name] !== "function"
      })
      expect(missing.map((e) => `${e.group ?? ""}.${e.method_name}`)).toEqual([])
    })
  }
})

describe("types (checked by tsc -p test)", () => {
  test("names follow Alien's TypeScript SDK", () => {
    const alien = new Alien({ apiKey: KEY, serverURL })
    expectTypeOf(alien.deployments.list).returns.resolves.toEqualTypeOf<platform.ListDeploymentsResponse>()
    expectTypeOf<AlienError["statusCode"]>().toEqualTypeOf<number | null>()
    // @ts-expect-error: groups are on the client, without a `.platform` level
    expect(alien.platform).toBeUndefined()
    // @ts-expect-error: only the async functions are bound, under their plain names
    expect(alien.deployments.list_async).toBeUndefined()
    // @ts-expect-error: the option is `serverURL`, as in Alien's SDK
    expect(() => new Alien({ apiKey: KEY, baseUrl: serverURL })).not.toThrow()
    // @ts-expect-error: the Manager's option is `token`, as in @alienplatform/commands
    expect(() => new AlienManager({ serverURL, bearer: TOKEN })).not.toThrow()
  })

  test("`platform` and `manager` hold types only", async () => {
    const body: platform.CreateSetupLinkRequest = { externalId: "cust-1", name: "Customer", project: "web" }
    expect(body.project).toBe("web")
    const exported = Object.keys(await import("../src/index.js"))
    expect(exported).not.toContain("platform")
    expect(exported).not.toContain("manager")
  })
})
