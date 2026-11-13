import { BamlError } from "@boundaryml/baml-bridge"
import * as sdk from "../generated/index.js"

/**
 * The error every Alien SDK call throws (docs/errors.md). Field names follow
 * the errors of Alien's TypeScript SDK (`statusCode`, `hint`, `requestId`).
 *
 * `cause` is `source`, the error the server names as this one's cause, so
 * anything that walks `cause` walks the server's chain. Without a source the
 * error has no `cause` property at all. The BAML runtime's own error, which
 * carried the throw, is not in the chain.
 */
export class AlienError extends Error {
  override readonly name = "AlienError"
  /** Machine-readable code: the server's own (`NOT_FOUND`) or the SDK's (`COMMUNICATION_ERROR`). */
  readonly code: string
  /** Whether retrying the call can help. */
  readonly retryable: boolean
  /** The HTTP response status, or `null` when there was no response. On a `source` level: the status that level names, if any. */
  readonly statusCode: number | null
  /** The server's remediation hint, if it sent one. */
  readonly hint: string | null
  /** The server's request ID (the error body's `requestId`), if it sent one. */
  readonly requestId: string | null
  /** The server's `internal` flag, or `null` when the error is not from a server's error body. */
  readonly internal: boolean | null
  /** The error that caused this one, from the server's `source` chain (at most 8 levels below the thrown error); also `cause`. */
  readonly source: AlienError | null
  /** Strings for debugging: `path`, and the server's context as JSON text in `details` (docs/errors.md). */
  readonly context: Readonly<Record<string, string>> | null

  constructor(value: sdk.errors.AlienError) {
    const source = value.source ? new AlienError(value.source) : null
    // No options at all without a source: `{ cause: undefined }` would still define `cause`.
    super(value.message, source ? { cause: source } : undefined)
    this.code = value.code
    this.retryable = value.retryable
    this.statusCode = value.http_status ?? null
    this.hint = value.hint ?? null
    this.requestId = value.request_id ?? null
    this.internal = value.internal ?? null
    this.source = source
    // A plain copy: the runtime's map has a null prototype.
    this.context = value.context ? { ...value.context } : null
  }
}

/** Converts a BAML-thrown AlienError; any other error is returned unchanged. */
function toAlienError(error: unknown): unknown {
  if (error instanceof BamlError && error.value instanceof sdk.errors.AlienError) {
    return new AlienError(error.value)
  }
  return error
}

export async function call<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn()
  } catch (error) {
    throw toAlienError(error)
  }
}

export function callSync<T>(fn: () => T): T {
  try {
    return fn()
  } catch (error) {
    throw toAlienError(error)
  }
}
