"""Benchmarks for the Python SDK (bench/run.py drives it and lists the modes).

Every mode prints one line, then waits for stdin EOF so the driver can read RSS.
"""

import json
import sys
import time

import alien_sdk
from baml_sdk import compat

ID = "mgr_0c29fq4a2yjb7kx3smwdgxlcmpqr"
INPUT = alien_sdk.CloudFormationLaunch(
    templateUrl="https://example.com/setup.yaml", region="us-east-1", stackName="alien-setup",
    parameters={"ExternalId": "customer-0001"})


def batches(call, count: int, n: int, warmup: int) -> list[float]:
    """Per-call nanoseconds of `call`, one value per batch, after `warmup` calls."""
    for _ in range(warmup):
        call()
    per_call = []
    for _ in range(count):
        start = time.perf_counter_ns()
        for _ in range(n):
            call()
        per_call.append((time.perf_counter_ns() - start) / n)
    return per_call


mode, args = sys.argv[1], sys.argv[2:]
if mode == "import":
    alien_sdk.Alien("alien_bench_key", server_url="http://127.0.0.1:9")
    line = "ready"
elif mode == "get":
    line = alien_sdk.Alien("alien_bench_key", server_url=args[0]).managers.get(ID).id
elif mode == "warm-get":
    alien = alien_sdk.Alien("alien_bench_key", server_url=args[0])
    line = json.dumps(batches(lambda: alien.managers.get(ID), int(args[1]), int(args[2]), 200))
elif mode == "page":
    alien = alien_sdk.Alien("alien_bench_key", server_url=args[0])
    size, warmup, n = int(args[1]), int(args[2]), int(args[3])
    ms = []
    for i in range(warmup + n):
        start = time.perf_counter_ns()
        page = alien.managers.list(limit=size)
        if i >= warmup:
            ms.append((time.perf_counter_ns() - start) / 1e6)
        if len(page) != size or page[-1].metrics is None or page[-1].metrics.cpuUsagePercent != 12.25:
            raise SystemExit("bad page")
    line = json.dumps(ms)
elif mode in ("warm-helper", "warm-noop"):
    call = (lambda: alien_sdk.cloudformation_launch_url(INPUT)) if mode == "warm-helper" else compat.newer_remote_sandbox
    line = json.dumps(batches(call, int(args[0]), int(args[1]), 1000))
else:
    raise SystemExit(f"unknown mode {mode}")
print(line, flush=True)
sys.stdin.read()
