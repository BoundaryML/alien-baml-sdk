"""Context for the SDK numbers: Python with no SDK.

  start                                  print a line
  page <base_url> <size> <warmup> <n>    urllib + json.loads of the same page, per-call ms (JSON)

Every mode prints one line, then waits for stdin EOF.
"""

import json
import sys
import time
import urllib.request

mode = sys.argv[1]
if mode == "start":
    print("ready", flush=True)
elif mode == "page":
    base, size, warmup, n = sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
    ms = []
    for i in range(warmup + n):
        start = time.perf_counter_ns()
        request = urllib.request.Request(f"{base}/v1/managers?limit={size}", headers={"Authorization": "Bearer alien_bench_key"})
        with urllib.request.urlopen(request) as response:
            page = json.loads(response.read())
        if i >= warmup:
            ms.append((time.perf_counter_ns() - start) / 1e6)
        if len(page) != size:
            raise SystemExit("bad page")
    print(json.dumps(ms), flush=True)
else:
    raise SystemExit(f"unknown mode {mode}")
sys.stdin.read()
