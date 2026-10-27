#!/usr/bin/env bash
# One command for every measurement (modes and method: bench/run.py):
#
#   ALIEN_REF=/path/to/alien bench/run.sh [run.py options]
#
# Builds everything (bench/build.sh), measures (bench/run.py, options such as
# --quick or --runs N pass through) and writes metrics/results.md (bench/render.py).
set -euo pipefail
BENCH="$(cd "$(dirname "$0")" && pwd)"
"$BENCH/build.sh"
python3 -I "$BENCH/run.py" "$@"
python3 -I "$BENCH/render.py"
