// Runs every conformance scenario (conformance/scenarios) through the facade.
import { mkdirSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { afterAll, beforeAll, describe, expect, test } from "vitest"
import { Alien, AlienError, AlienManager, cloudFormationLaunchUrl, shutdown } from "../src/index.js"
import * as sdk from "../generated/index.js"
import { call } from "../src/errors.js"
import { type Call, loadScenarios, type OperationCall, Stub } from "./stub.js"

// Scenario arguments are JSON. Request bodies are passed as the plain objects
// they are (test/facade.test.ts covers generated classes).
type Args = Record<string, any>

function alien(client: Args): Alien {
  return new Alien({
    apiKey: client.api_key,
    serverURL: client.base_url,
    workspace: client.workspace ?? undefined,
    timeoutMs: client.timeout_ms,
  })
}

function alienManager(client: Args): AlienManager {
  return new AlienManager({ serverURL: client.base_url, token: client.token, timeoutMs: client.timeout_ms })
}

function platformClient(client: Args): sdk.platform.PlatformClient {
  return new sdk.platform.PlatformClient({
    base_url: client.base_url,
    api_key: client.api_key,
    workspace: client.workspace,
    timeout_ms: client.timeout_ms,
  })
}

function managerClient(client: Args): sdk.manager.ManagerClient {
  return new sdk.manager.ManagerClient({ base_url: client.base_url, token: client.token, timeout_ms: client.timeout_ms })
}

/**
 * A generated operation (`call.op`, conformance/README.md): `platform.managers.list`
 * is `sdk.platform.managers.list` (async), called with its required arguments in
 * order and the optional ones that are set in an options object.
 */
function operation(loaded: OperationCall): Promise<unknown> {
  const [service, ...path] = loaded.op.split(".")
  let target: any = sdk
  for (const part of [service, ...path.slice(0, -1)]) target = target?.[part]
  const fn = target?.[path.at(-1)!]
  if (typeof fn !== "function") throw new Error(`no generated TypeScript function for ${loaded.op}`)
  const client = service === "manager" ? managerClient(loaded.args.client) : platformClient(loaded.args.client)
  const required = loaded.params.filter((p) => !p.optional).map((p) => loaded.args[p.name])
  const options = Object.fromEntries(loaded.params.filter((p) => p.optional && p.name in loaded.args).map((p) => [p.name, loaded.args[p.name]]))
  return call(() => fn(client, ...required, ...(Object.keys(options).length > 0 ? [options] : [])))
}

/** A compat function that takes the scenario's `json` argument. */
function compatJson(fn: (json: string) => Promise<unknown>): (a: Args) => Promise<unknown> {
  return (a) => call(() => fn(a.json))
}

const calls: Record<string, (a: Args) => Promise<unknown>> = {
  "platform.createSetupLink": (a) => alien(a.client).setupLinks.create(a.request),
  "platform.getProjectTemplateUrls": (a) => alien(a.client).projects.getTemplateUrls(a.idOrName),
  "platform.bootstrapCommands": (a) => alien(a.client).commands.bootstrap(a.request),
  "platform.getManager": (a) => alien(a.client).managers.get(a.id),
  "manager.getCommandStatus": (a) => alienManager(a.client).commands.getCommandStatus(a.command_id),
  "setup.cloudformationLaunchUrl": async (a) => cloudFormationLaunchUrl(a.input),
  "setup.launchUrlForProject": (a) =>
    alien(a.client).setup.launchUrlForProject(a.project, a.region, a.stackName, a.parameters),
  "compat.acceptKeySetup": (a) => call(() => sdk.compat.accept_key_setup(platformClient(a.client), a.request)),
  "compat.decodeProjectPage": compatJson(sdk.compat.decode_project_page),
  "compat.decodeMaterialization": compatJson(sdk.compat.decode_materialization),
  "compat.storedSecretPresence": compatJson(sdk.compat.stored_secret_presence),
  "compat.reencodeConfig": compatJson(sdk.compat.reencode_config),
  "compat.roundtripStackByPlatform": compatJson(sdk.compat.roundtrip_stack_by_platform),
  "compat.roundtripEvent": compatJson(sdk.compat.roundtrip_event),
  "compat.roleScope": compatJson(sdk.compat.role_scope),
  "compat.decodeRemoteAccess": compatJson(sdk.compat.decode_remote_access),
}

/** An error as the stub compares it: every field, under its BAML name, `source` level by level. */
function errorOutcome(error: AlienError): Record<string, unknown> {
  return {
    code: error.code,
    message: error.message,
    retryable: error.retryable,
    http_status: error.statusCode,
    context: error.context,
    hint: error.hint,
    request_id: error.requestId,
    internal: error.internal,
    source: error.source ? errorOutcome(error.source) : null,
  }
}

async function outcomeOf(loaded: Call): Promise<{ ok?: unknown; error?: unknown; raw?: unknown }> {
  let run: () => Promise<unknown>
  if ("op" in loaded) {
    run = () => operation(loaded)
  } else {
    const named = calls[loaded.fn]
    if (!named) throw new Error(`no TypeScript binding for scenario fn ${loaded.fn}`)
    run = () => named(loaded.args)
  }
  try {
    const value = await run()
    return { ok: value === undefined ? null : JSON.parse(JSON.stringify(value)), raw: value }
  } catch (error) {
    if (error instanceof AlienError) return { error: errorOutcome(error) }
    throw error
  }
}

const scenarios = loadScenarios()
let stub: Stub

beforeAll(async () => {
  stub = await Stub.start()
})
afterAll(async () => {
  stub?.stop()
  // Flush telemetry: Vitest ends workers without Node's beforeExit.
  await shutdown()
})

describe("conformance", () => {
  test("found scenarios", () => {
    expect(scenarios.length).toBeGreaterThan(0)
  })

  for (const scenario of scenarios) {
    test(scenario.id, async () => {
      const loaded = await stub.load(scenario)
      const { raw, ...outcome } = await outcomeOf(loaded)
      const verdict = await stub.result(outcome)
      const out = process.env.CONFORMANCE_OUT
      if (out) {
        mkdirSync(join(out, "typescript"), { recursive: true })
        writeFileSync(join(out, "typescript", `${scenario.id}.json`), JSON.stringify(verdict.normalized) + "\n")
        if (scenario.capture_raw) writeFileSync(join(out, "typescript", `${scenario.id}.txt`), String(raw))
      }
      expect(verdict.problems, verdict.problems.join("\n")).toEqual([])
      expect(verdict.ok).toBe(true)
    })
  }
})
