// Benchmarks for the TypeScript SDK (bench/run.py drives it and lists the modes).
// Every mode prints one line, then waits for stdin EOF so the driver can read RSS.
import { Alien, cloudFormationLaunchUrl } from "../../sdk/typescript/dist/src/index.js"
import * as sdk from "../../sdk/typescript/dist/baml_sdk/index.js"

const ID = "mgr_0c29fq4a2yjb7kx3smwdgxlcmpqr"
const input = { templateUrl: "https://example.com/setup.yaml", region: "us-east-1", stackName: "alien-setup", parameters: { ExternalId: "customer-0001" } }
const [mode, ...args] = process.argv.slice(2)

function done(line) {
  console.log(line)
  process.stdin.resume()
  process.stdin.on("end", () => process.exit(0))
}

// Per-call nanoseconds of `call`, one value per batch, after `warmup` calls.
async function batches(call, count, n, warmup) {
  for (let i = 0; i < warmup; i++) await call()
  const perCall = []
  for (let batch = 0; batch < count; batch++) {
    const start = process.hrtime.bigint()
    for (let i = 0; i < n; i++) await call()
    perCall.push(Number(process.hrtime.bigint() - start) / n)
  }
  return perCall
}

// The same for a synchronous call.
function batchesSync(call, count, n, warmup) {
  for (let i = 0; i < warmup; i++) call()
  const perCall = []
  for (let batch = 0; batch < count; batch++) {
    const start = process.hrtime.bigint()
    for (let i = 0; i < n; i++) call()
    perCall.push(Number(process.hrtime.bigint() - start) / n)
  }
  return perCall
}

if (mode === "import") {
  new Alien({ apiKey: "alien_bench_key", serverURL: "http://127.0.0.1:9" })
  done("ready")
} else if (mode === "get") {
  const alien = new Alien({ apiKey: "alien_bench_key", serverURL: args[0] })
  done((await alien.managers.get(ID)).id)
} else if (mode === "warm-get") {
  const alien = new Alien({ apiKey: "alien_bench_key", serverURL: args[0] })
  done(JSON.stringify(await batches(() => alien.managers.get(ID), Number(args[1]), Number(args[2]), 200)))
} else if (mode === "page") {
  const alien = new Alien({ apiKey: "alien_bench_key", serverURL: args[0] })
  const [size, warmup, n] = args.slice(1).map(Number)
  const ms = []
  for (let i = 0; i < warmup + n; i++) {
    const start = process.hrtime.bigint()
    const page = await alien.managers.list({ limit: size })
    if (i >= warmup) ms.push(Number(process.hrtime.bigint() - start) / 1e6)
    if (page.length !== size || page[size - 1].metrics?.cpuUsagePercent !== 12.25) throw new Error("bad page")
  }
  done(JSON.stringify(ms))
} else if (mode === "warm-helper" || mode === "warm-noop") {
  const call = mode === "warm-helper" ? () => cloudFormationLaunchUrl(input) : () => sdk.compat.newer_remote_sandbox()
  done(JSON.stringify(batchesSync(call, Number(args[0]), Number(args[1]), 1000)))
} else {
  throw new Error(`unknown mode ${mode}`)
}
