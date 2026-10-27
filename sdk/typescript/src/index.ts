// Alien SDK for TypeScript: a thin facade over the generated BAML SDK
// (../baml_sdk). It only builds clients, renames, converts errors and picks the
// public surface; every behavior lives in the BAML project (baml/baml_src).
// Client options and error fields use the names of Alien's TypeScript SDK
// (@alienplatform/platform-api); the Manager's `token` is the name
// @alienplatform/commands uses.
import { shutdownRuntime } from "@boundaryml/baml-bridge"
import * as sdk from "../baml_sdk/index.js"
import { type Bound, type BoundOperations, type Groups, bind } from "./bind.js"
import { AlienError, call, callSync } from "./errors.js"

export { AlienError } from "./errors.js"
/**
 * Types of the Platform and Manager APIs (request and response shapes). Types
 * only: request bodies are plain objects, as in Alien's TypeScript SDK, and the
 * generated functions (which skip the facade's error conversion) stay private.
 */
export type * as platform from "../baml_sdk/platform/index.js"
export type * as manager from "../baml_sdk/manager/index.js"

export type CloudFormationLaunch = sdk.setup.CloudFormationLaunch

/** The Platform API URL used when `serverURL` is not set. */
export const DEFAULT_SERVER_URL = "https://api.alien.dev"

export interface AlienOptions {
  /**
   * Platform API key, sent as `Authorization: Bearer <apiKey>`. Defaults to the
   * `ALIEN_API_KEY` environment variable; one of the two is required. A
   * function is called before every request.
   */
  apiKey?: string | (() => Promise<string>)
  /** Platform API URL. Defaults to https://api.alien.dev. */
  serverURL?: string
  /**
   * Workspace name, sent on every operation that accepts it. Defaults to the
   * `ALIEN_WORKSPACE` environment variable; optional for workspace-scoped API keys.
   */
  workspace?: string
  /**
   * Limit in milliseconds for each HTTP request, from connecting to reading the
   * whole response, as in Alien's TypeScript SDK. Unset: BAML's default of 5
   * minutes; 0 or negative: no limit. A request that runs out fails with
   * `AlienError` `COMMUNICATION_ERROR` (retryable). A per-call
   * `{ $baml: { timeoutMs } }` limits the whole call as well: the shorter one ends it.
   */
  timeoutMs?: number
}

export interface ManagerOptions {
  /** The Manager's URL, for example `managerUrl` from `alien.commands.bootstrap`. */
  serverURL: string
  /** Bearer token, for example `token` from `alien.commands.bootstrap`; sent on every Manager operation when set. */
  token?: string
  /**
   * Limit in milliseconds for each HTTP request, from connecting to reading the
   * whole response, as in Alien's TypeScript SDK. Unset: BAML's default of 5
   * minutes; 0 or negative: no limit. A request that runs out fails with
   * `AlienError` `COMMUNICATION_ERROR` (retryable). A per-call
   * `{ $baml: { timeoutMs } }` limits the whole call as well: the shorter one ends it.
   */
  timeoutMs?: number
}

type PlatformApi = typeof sdk.platform
type ManagerApi = typeof sdk.manager

/** Returns the Platform client to call with; called before every request. */
type PlatformClientSource = () => sdk.platform.PlatformClient | Promise<sdk.platform.PlatformClient>

/** A Platform API group bound to a client, for example the type of `alien.deployments`. */
export type PlatformGroup<G extends Groups<PlatformApi>> = Bound<PlatformApi[G]>
/** A Manager API group bound to a client, for example the type of `manager.commands`. */
export type ManagerGroup<G extends Groups<ManagerApi>> = Bound<ManagerApi[G]>

// Groups are typed by name, so editors show `PlatformGroup<"deployments">`
// rather than the path of a generated module.
type PlatformMembers = BoundOperations<PlatformApi> & { readonly [G in Groups<PlatformApi>]: PlatformGroup<G> }
type ManagerMembers = BoundOperations<ManagerApi> & { readonly [G in Groups<ManagerApi>]: ManagerGroup<G> }

/** The variable's value; unset and empty are both `undefined`. */
function env(name: string): string | undefined {
  return process.env[name] || undefined
}

/** `timeoutMs` as the generated clients take it: `null` when unset. */
function timeoutOption(timeoutMs: number | undefined): number | null {
  if (timeoutMs == null) return null
  if (!Number.isSafeInteger(timeoutMs)) throw new TypeError(`timeoutMs must be an integer number of milliseconds, got ${timeoutMs}`)
  return timeoutMs
}

/** An `INVALID_REQUEST` error: the SDK refuses to send a request without an API key. */
function noApiKey(message: string): AlienError {
  return new AlienError(new sdk.errors.AlienError({ code: "INVALID_REQUEST", message, retryable: false }))
}

/**
 * Returns the Platform client to call with: built once from a string key, or
 * before every request from the value of an `apiKey` function.
 */
function platformClient(
  apiKey: string | (() => Promise<string>),
  make: (apiKey: string) => sdk.platform.PlatformClient,
): PlatformClientSource {
  if (typeof apiKey === "string") {
    const client = make(apiKey)
    return () => client
  }
  return async () => {
    const value: unknown = await apiKey()
    if (typeof value !== "string" || value === "") throw noApiKey("no API key: the apiKey function returned none")
    return make(value)
  }
}

/**
 * A Platform API client: `new Alien({ apiKey }).<group>.<method>(...)`, as in
 * Alien's TypeScript SDK. `await shutdown()` before the process ends early
 * (`process.exit()`, a signal): see {@link shutdown}.
 */
export interface Alien extends PlatformMembers {}
export class Alien {
  /** Setup helpers that call the Platform API. */
  readonly setup: ProjectSetup

  constructor(options: AlienOptions = {}) {
    const apiKey = options.apiKey ?? env("ALIEN_API_KEY")
    if (!apiKey) throw noApiKey("no API key: pass apiKey or set ALIEN_API_KEY")
    const serverURL = options.serverURL ?? DEFAULT_SERVER_URL
    const workspace = options.workspace ?? env("ALIEN_WORKSPACE")
    const timeout_ms = timeoutOption(options.timeoutMs)
    const client = platformClient(apiKey, (key) =>
      new sdk.platform.PlatformClient({ base_url: serverURL, api_key: key, workspace, timeout_ms }),
    )
    Object.assign(this, bind(sdk.platform, client))
    this.setup = new ProjectSetup(client)
  }
}

/**
 * A Manager API client: `new AlienManager({ serverURL, token }).<group>.<method>(...)`.
 * `await shutdown()` before the process ends early (`process.exit()`, a
 * signal): see {@link shutdown}.
 */
export interface AlienManager extends ManagerMembers {}
export class AlienManager {
  constructor(options: ManagerOptions) {
    if (typeof options?.serverURL !== "string") throw new TypeError("AlienManager needs `serverURL`")
    // An empty token sends no Authorization header.
    const client = new sdk.manager.ManagerClient({
      base_url: options.serverURL,
      token: options.token || null,
      timeout_ms: timeoutOption(options.timeoutMs),
    })
    Object.assign(this, bind(sdk.manager, () => client))
  }
}

/** Setup helpers that need the Platform API: `alien.setup`. */
export class ProjectSetup {
  readonly #client: PlatformClientSource
  /** Created by `Alien`; use `alien.setup`. */
  constructor(client: PlatformClientSource) {
    this.#client = client
  }

  /** Fetches the project's AWS template and builds a quick-create link for it. */
  launchUrlForProject(
    project: string,
    region: string,
    stackName: string,
    parameters: Record<string, string> = {},
  ): Promise<string> {
    return call(async () =>
      sdk.setup.launch_url_for_project_async(await this.#client(), project, region, stackName, parameters),
    )
  }
}

/** Builds an AWS CloudFormation quick-create link from a template URL. */
export function cloudFormationLaunchUrl(input: CloudFormationLaunch): string {
  return callSync(() => sdk.setup.cloudformation_launch_url(input))
}

/**
 * Shuts the SDK's runtime down: waits for calls in flight and seals the
 * telemetry recording. SDK calls made afterwards fail.
 *
 * The runtime does this by itself when Node's event loop runs empty
 * (`beforeExit`). Unless telemetry is off (`ALIEN_TELEMETRY=off`), await
 * `shutdown()` before the process ends any other way: `process.exit()`, a signal handler,
 * a test runner ending its workers, a serverless platform ending the process.
 * Otherwise the recording is not sealed (docs/runtime.md).
 */
export async function shutdown(): Promise<void> {
  await shutdownRuntime()
}
