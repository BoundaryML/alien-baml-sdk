#!/usr/bin/env node
// Local stand-in for a telemetry endpoint (docs/telemetry.md). Point BOUNDARY_API_URL at
// it and it records every connection attempt and every request it receives,
// one JSON line each, to --log. It answers every request with 503, so nothing
// is ever accepted.
//
//   node sink.mjs --log path/to/sink.jsonl
//
// Prints `READY http://127.0.0.1:<port>` once listening.

import { appendFileSync, writeFileSync } from "node:fs"
import { createServer } from "node:http"

const logIndex = process.argv.indexOf("--log")
if (logIndex < 0 || !process.argv[logIndex + 1]) throw new Error("usage: sink.mjs --log <file>")
const logPath = process.argv[logIndex + 1]
writeFileSync(logPath, "")

const record = (entry) => appendFileSync(logPath, JSON.stringify({ t: new Date().toISOString(), ...entry }) + "\n")

const server = createServer((req, res) => {
  const chunks = []
  req.on("data", (c) => chunks.push(c))
  req.on("end", () => {
    record({
      kind: "request",
      method: req.method,
      path: req.url,
      authorization: req.headers.authorization ?? null,
      bytes: Buffer.concat(chunks).length,
    })
    res.writeHead(503, { "content-type": "application/json" })
    res.end(JSON.stringify({ code: "SINK", message: "telemetry sink: nothing is accepted" }))
  })
})
server.on("connection", (socket) => record({ kind: "connection", remote: socket.remotePort }))
server.listen(0, "127.0.0.1", () => process.stdout.write(`READY http://127.0.0.1:${server.address().port}\n`))
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => server.close(() => process.exit(0)))
