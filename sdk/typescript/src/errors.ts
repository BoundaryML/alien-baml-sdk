import { BamlError } from "@boundaryml/baml-bridge"
import * as sdk from "../baml_sdk/index.js"

/**
 * The error every Alien SDK call throws (docs/errors.md). Field names follow
 * the errors of Alien's TypeScript SDK (`statusCode`, `hint`, `requestId`).
 */
export class AlienError extends Error {
  override readonly name = "AlienError"
  /** Machine-readable code: the server's own (`NOT_FOUND`) or the SDK's (`COMMUNICATION_ERROR`). */
  readonly code: string
  /** Whether retrying the call can help. */
  readonly retryable: boolean
  /** The HTTP response status, or `null` when there was no response. */
  readonly statusCode: number | null
  /** The server's remediation hint (`context.hint`), if it sent one. */
  readonly hint: string | null
  /** The server's request ID (`context.requestId`), if it sent one. */
  readonly requestId: string | null
  /** Strings for debugging: `path`, and the server's context as JSON text in `details` (docs/errors.md). */
  readonly context: Readonly<Record<string, string>> | null

  constructor(value: sdk.errors.AlienError, options?: { cause?: unknown }) {
    super(value.message, options)
    this.code = value.code
    this.retryable = value.retryable
    this.statusCode = value.http_status ?? null
    // A plain copy: the runtime's map has a null prototype.
    this.context = value.context ? { ...value.context } : null
    this.hint = this.context?.hint ?? null
    this.requestId = this.context?.requestId ?? null
  }
}

/** Converts a BAML-thrown AlienError; any other error is returned unchanged. */
function toAlienError(error: unknown): unknown {
  if (error instanceof BamlError && error.value instanceof sdk.errors.AlienError) {
    return new AlienError(error.value, { cause: error })
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
