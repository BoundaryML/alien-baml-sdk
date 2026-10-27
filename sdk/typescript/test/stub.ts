// Starts conformance/stub-server and talks to its control endpoints.
import { spawn, type ChildProcess } from "node:child_process"
import { readdirSync, readFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { createInterface } from "node:readline"
import { fileURLToPath } from "node:url"

export const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..")

/** A named call, bound by hand in each harness. */
export interface NamedCall {
  fn: string
  args: Record<string, any>
}

/** A generated operation, called through one generic dispatch (conformance/README.md). */
export interface OperationCall {
  op: string
  params: { name: string; type: string; optional: boolean }[]
  args: Record<string, any>
}

export type Call = NamedCall | OperationCall

export interface Scenario {
  id: string
  phase: number
  call: Call
  capture_raw?: boolean
}

/** Scenarios with phase <= CONFORMANCE_PHASE (every phase when unset). */
export function loadScenarios(): Scenario[] {
  const maxPhase = process.env.CONFORMANCE_PHASE ? Number(process.env.CONFORMANCE_PHASE) : Number.POSITIVE_INFINITY
  const dir = join(repoRoot, "conformance", "scenarios")
  return readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .sort()
    .map((f) => {
      const scenario = JSON.parse(readFileSync(join(dir, f), "utf8")) as Scenario
      // A missing phase would drop the scenario silently.
      if (typeof scenario.phase !== "number") throw new Error(`${f}: \`phase\` must be a number`)
      return scenario
    })
    .filter((s) => s.phase <= maxPhase)
}

export class Stub {
  private constructor(private readonly child: ChildProcess, readonly base: string) {}

  static async start(): Promise<Stub> {
    const child = spawn(process.execPath, [join(repoRoot, "conformance", "stub-server", "server.mjs")], {
      stdio: ["ignore", "pipe", "inherit"],
    })
    const lines = createInterface({ input: child.stdout! })
    const base = await new Promise<string>((ok, fail) => {
      child.once("exit", (code) => fail(new Error(`stub server exited early with ${code}`)))
      lines.once("line", (line) => {
        if (!line.startsWith("READY ")) fail(new Error(`unexpected stub output: ${line}`))
        else ok(line.slice("READY ".length))
      })
    })
    return new Stub(child, base)
  }

  async load(scenario: Scenario): Promise<Scenario["call"]> {
    const res = await this.post("/__load", { scenario })
    return res.call
  }

  async result(outcome: unknown): Promise<{ ok: boolean; problems: string[]; normalized: unknown }> {
    return this.post("/__result", outcome)
  }

  private async post(path: string, body: unknown): Promise<any> {
    const res = await fetch(this.base + path, { method: "POST", body: JSON.stringify(body) })
    const json = await res.json()
    if (!res.ok) throw new Error(`stub ${path} failed: ${JSON.stringify(json)}`)
    return json
  }

  stop(): void {
    this.child.kill()
  }
}
