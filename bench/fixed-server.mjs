#!/usr/bin/env node
// Zero-latency Platform API stand-in for benchmarks, on 127.0.0.1 only.
// Prints `READY http://127.0.0.1:<port>`.
//   GET /v1/managers/{id}      the same Manager every time
//   GET /v1/managers?limit=N   an array of N Managers (default 20), each with its own id
//   GET /stats                 {"connections", "requests"} accepted so far, this request
//                              and its connection included: whether a client reuses connections
import { createServer } from "node:http"

const MANAGER = {
  id: "mgr_0c29fq4a2yjb7kx3smwdgxlcmpqr", name: "primary", targets: ["aws"], cloud: "aws", region: "us-east-1",
  setupStatus: "active", url: "https://manager.example.com",
  managementConfigs: { aws: { managingRoleArn: "arn:aws:iam::000000000000:role/example", platform: "aws" } },
  isSystem: false, workspaceId: "ws_AAAAAAAAAAAAAAAAAAAAAAAA", status: "healthy", version: "1.2.3",
  metrics: { activeDeployments: 3, pendingDeployments: 0, memoryUsageMb: 512.5, cpuUsagePercent: 12.25 },
  lastHeartbeatAt: null, logsDatabaseId: null, managedDeploymentCount: 3, defaultProjectCount: 1,
  createdAt: "2026-10-01T00:00:00Z",
}
const one = JSON.stringify(MANAGER)
const pages = new Map()
function page(n) {
  if (!pages.has(n)) {
    const items = Array.from({ length: n }, (_, i) => ({ ...MANAGER, id: `mgr_${String(i).padStart(28, "0")}` }))
    pages.set(n, JSON.stringify(items))
  }
  return pages.get(n)
}

function send(res, status, body) {
  res.writeHead(status, { "content-type": "application/json", "content-length": Buffer.byteLength(body) })
  res.end(body)
}

let connections = 0
let requests = 0
const server = createServer((req, res) => {
  requests++
  req.resume()
  req.on("end", () => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1")
    if (req.method === "GET" && url.pathname === "/stats") {
      send(res, 200, JSON.stringify({ connections, requests }))
    } else if (req.method === "GET" && /^\/v1\/managers\/[^/]+$/.test(url.pathname)) {
      send(res, 200, one)
    } else if (req.method === "GET" && url.pathname === "/v1/managers") {
      const limit = Number(url.searchParams.get("limit") ?? "20")
      if (!Number.isInteger(limit) || limit < 0 || limit > 100_000) {
        send(res, 400, JSON.stringify({ code: "BAD_REQUEST", message: `bad limit ${url.searchParams.get("limit")}`, internal: false }))
      } else {
        send(res, 200, page(limit))
      }
    } else {
      send(res, 404, JSON.stringify({ code: "NOT_FOUND", message: `no route ${req.method} ${req.url}`, internal: false }))
    }
  })
})
server.on("connection", () => connections++)
server.keepAliveTimeout = 60_000
server.listen(0, "127.0.0.1", () => process.stdout.write(`READY http://127.0.0.1:${server.address().port}\n`))
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => process.exit(0))
