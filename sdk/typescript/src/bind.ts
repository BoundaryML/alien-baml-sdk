// Binds a generated BAML namespace to a client, without per-operation code:
// every `<name>_async(client, ...args)` becomes `<name>(...args)`, every group
// namespace is bound the same way, and BAML-thrown AlienErrors become
// `AlienError`. Types follow the generated signatures.
import { call } from "./errors.js"

type Fn = (...args: never[]) => unknown
type Class = abstract new (...args: never[]) => unknown

/** Operation names: each `<name>_async` has a synchronous twin `<name>` with the same docs. */
type Operations<NS> = { [K in keyof NS]: K extends string ? (`${K}_async` extends keyof NS ? K : never) : never }[keyof NS]

/** Group names: the nested namespaces (objects that are neither functions nor classes). */
export type Groups<NS> = { [K in keyof NS]: NS[K] extends Fn | Class ? never : NS[K] extends object ? K : never }[keyof NS]

type WithoutClient<F> = F extends (client: never, ...args: infer A) => Promise<infer R> ? (...args: A) => Promise<R> : never

// Mapping over `K extends keyof NS` keeps each key's declaration, so editors
// show the docs of the generated `<name>` on the bound `<name>`.
type PickOperations<NS, K extends keyof NS> = {
  [P in K]: P extends string ? WithoutClient<NS[`${P}_async` & keyof NS]> : never
}

/** The operations of a generated namespace `NS`, bound to a client. */
export type BoundOperations<NS> = PickOperations<NS, Operations<NS>>

/** A generated namespace `NS` bound to a client: its operations, and each group bound the same way. */
export type Bound<NS> = BoundOperations<NS> & { readonly [G in Groups<NS>]: Bound<NS[G]> }

// Generated modules export functions, classes and group namespaces; only the
// namespaces are objects. (Not `[object Module]`: some bundlers do not tag
// namespace objects.)
function isNamespace(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}

/** `client` is called before every call; it returns the client to call with. */
export function bind<NS>(namespace: NS, client: () => unknown): Bound<NS> {
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(namespace as Record<string, unknown>)) {
    if (key.endsWith("_async") && typeof value === "function") {
      const fn = value as (client: unknown, ...args: unknown[]) => Promise<unknown>
      out[key.slice(0, -"_async".length)] = (...args: unknown[]) => call(async () => fn(await client(), ...args))
    } else if (isNamespace(value)) {
      out[key] = bind(value, client)
    }
  }
  return out as Bound<NS>
}
