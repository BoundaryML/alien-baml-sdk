// Context for the SDK numbers: Node with no SDK.
//   start                          print a line
//   page <base_url> <size> <warmup> <n>   fetch + JSON.parse of the same page, per-call ms (JSON)
// Every mode prints one line, then waits for stdin EOF.
const [mode, ...args] = process.argv.slice(2)

function done(line) {
  console.log(line)
  process.stdin.resume()
  process.stdin.on("end", () => process.exit(0))
}

if (mode === "start") {
  done("ready")
} else if (mode === "page") {
  const [size, warmup, n] = args.slice(1).map(Number)
  const ms = []
  for (let i = 0; i < warmup + n; i++) {
    const start = process.hrtime.bigint()
    const response = await fetch(`${args[0]}/v1/managers?limit=${size}`, { headers: { authorization: "Bearer alien_bench_key" } })
    if (!response.ok) throw new Error(`status ${response.status}`)
    const page = await response.json()
    if (i >= warmup) ms.push(Number(process.hrtime.bigint() - start) / 1e6)
    if (page.length !== size) throw new Error("bad page")
  }
  done(JSON.stringify(ms))
} else {
  throw new Error(`unknown mode ${mode}`)
}
