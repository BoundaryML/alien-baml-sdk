// Benchmark for Alien's current TypeScript SDK (@alienplatform/platform-api,
// built by bench/build.sh from the Alien repository). Same modes as
// bench/typescript/bench.mjs, plus:
//   get-standalone <base_url>   one call through AlienCore + funcs/managersGet (tree-shakable form)
// Every mode prints one line, then waits for stdin EOF.
const [mode, ...args] = process.argv.slice(2)
const ID = "mgr_0c29fq4a2yjb7kx3smwdgxlcmpqr"
const root = "../../.cache/incumbents/platform-api/esm"

function done(line) {
  console.log(line)
  process.stdin.resume()
  process.stdin.on("end", () => process.exit(0))
}

async function client(baseUrl) {
  const { Alien } = await import(`${root}/index.js`)
  return new Alien({ apiKey: "alien_bench_key", serverURL: baseUrl })
}

if (mode === "import") {
  await client("http://127.0.0.1:9")
  done("ready")
} else if (mode === "get") {
  const alien = await client(args[0])
  done((await alien.managers.get({ id: ID })).id)
} else if (mode === "get-standalone") {
  const { AlienCore } = await import(`${root}/core.js`)
  const { managersGet } = await import(`${root}/funcs/managersGet.js`)
  const result = await managersGet(new AlienCore({ apiKey: "alien_bench_key", serverURL: args[0] }), { id: ID })
  if (!result.ok) throw result.error
  done(result.value.id)
} else if (mode === "warm-get") {
  const alien = await client(args[0])
  const [count, n] = [Number(args[1]), Number(args[2])]
  for (let i = 0; i < 200; i++) await alien.managers.get({ id: ID })
  const perCall = []
  for (let batch = 0; batch < count; batch++) {
    const start = process.hrtime.bigint()
    for (let i = 0; i < n; i++) await alien.managers.get({ id: ID })
    perCall.push(Number(process.hrtime.bigint() - start) / n)
  }
  done(JSON.stringify(perCall))
} else if (mode === "page") {
  const alien = await client(args[0])
  const [size, warmup, n] = args.slice(1).map(Number)
  const ms = []
  for (let i = 0; i < warmup + n; i++) {
    const start = process.hrtime.bigint()
    const page = await alien.managers.list({ limit: size })
    if (i >= warmup) ms.push(Number(process.hrtime.bigint() - start) / 1e6)
    if (page.length !== size || page[size - 1].metrics?.cpuUsagePercent !== 12.25) throw new Error("bad page")
  }
  done(JSON.stringify(ms))
} else {
  throw new Error(`unknown mode ${mode}`)
}
