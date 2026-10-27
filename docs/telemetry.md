# Telemetry

The BAML runtime inside every SDK records what each SDK function does
(spans, timings, HTTP calls) without instrumentation code. It is on by
default, at `medium`; `ALIEN_TELEMETRY=off` turns it off.

## The switch

| `ALIEN_TELEMETRY` | Effect |
|---|---|
| unset or empty | `medium`, BAML's default |
| `off` | Nothing is recorded and no connection is made |
| `low` | Functions are not observed unless annotated; HTTP calls are not recorded |
| `medium` | Timing for every function, and a span per HTTP call |
| `high` | A span per function (without capturing extra values), and a span per HTTP call |

Values are trimmed and case-insensitive. `false`, `0` and `no` also mean
`off`; `true`, `1`, `yes` and `on` mean `medium`. Any other value turns
telemetry off, silently. No telemetry setting stops the SDK from loading: an
empty `BOUNDARY_API_KEY` counts as unset, and a malformed `BOUNDARY_API_URL`
makes the runtime record locally.
`BAML_TELEMETRY` has no effect on these SDKs: `baml/baml.toml` renames it
(`[bridge.env_var_names] BAML_TELEMETRY = "ALIEN_TELEMETRY"`), and the name is
baked into the generated code, so changing it means regenerating.

Where recordings go:

- `BOUNDARY_API_KEY=local`: local files only, under `$BAML_HOME/btel/`
  (`BAML_HOME` defaults to `~/.baml`).
- Any other `BOUNDARY_API_KEY`: uploaded to `BOUNDARY_API_URL`.
- No key: uploaded with a saved Boundary login for that endpoint if there is
  one ([gaps B3](gaps.md#b3)), otherwise local files. This is what an end user
  gets by default: T4 checks that the full suite then writes only under
  `$BAML_HOME/btel/` and connects nowhere.

Nothing deletes local recordings. Each process adds at least one recording
file, and each call about 1.2 KB (measured with the Python SDK: 200 calls in
one process wrote 5 files, 243 KB; ten processes of 20 calls each wrote 18
files, 324 KB). Delete `$BAML_HOME/btel/` to reclaim the space. With
`ALIEN_TELEMETRY=off` nothing is written under `$BAML_HOME` and no connection
is made.

The SDK never prints telemetry warnings: when the endpoint cannot be reached or
rejects the key, telemetry is dropped silently and the program continues
(`baml/baml.toml`: `on_initial_telemetry_failure = "ignore"` under `[bridge]`;
BAML's default prints a warning on stderr). Such a process takes about 5 s
longer to exit, whether the connection is refused or never answered, while
the runtime tries to deliver what it recorded.

Read local recordings with `baml query --local --from <dir>`, which reads
`<dir>/.baml/btel/`: `--from ~` for the default `BAML_HOME` (`~/.baml`),
`--from /x` for `BAML_HOME=/x/.baml`.

**HTTP bodies are never recorded.** Alien's responses carry Manager tokens
and setup-link tokens, so `baml/baml.toml` sets `http_bodies = false` under
`[bridge]` ([gaps B2](gaps.md#b2)). A span per HTTP call still records the
method, the URL and headers (query values and most header values hashed), the
status, the timing and whether it failed, but not its error, which can quote a
body. The SDK's own functions record timing (`medium`) or a span without
argument or result values (`high`). T3 checks that no fixture secret is
recorded at either level.

## Flushing

Recordings are flushed about once a second and sealed when the runtime shuts
down. Python and Rust shut it down when the process exits normally,
TypeScript when Node's event loop runs empty, Go never on its own. Unless
telemetry is off, call the SDK's shutdown wherever that does not happen
(`process.exit()`, signals, aborts, every Go `main`): `await shutdown()`,
`alien_sdk.shutdown()`, `alien.Shutdown()`, `alien_sdk::shutdown()`. Details
in [runtime.md](runtime.md#shutting-down).

## Checks

`scripts/verify.sh` runs these in all four languages, each with a fresh
`BAML_HOME` and a local sink (`telemetry/sink.mjs`) as `BOUNDARY_API_URL` that
records every connection attempt. "Probe" is the README quickstart (the setup
helper only, so its recordings cannot contain a fixture secret); "suite" is
the full conformance suite.

| ID | Check | Result |
|---|---|---|
| T1 | `ALIEN_TELEMETRY=high` records (probe); `ALIEN_TELEMETRY=off` with `BAML_TELEMETRY=high` records nothing; `ALIEN_TELEMETRY=medium` with `BAML_TELEMETRY=off` records | Pass, all four languages |
| T2 | `ALIEN_TELEMETRY=off` with a Boundary API key (the fake `boundaryApiKey` from `conformance/secrets.json`), full suite: no file under `BAML_HOME`, no sink connection. Control: the probe with `ALIEN_TELEMETRY` unset and the same key does reach the sink | Pass, all four languages |
| T3 | `ALIEN_TELEMETRY` unset (the default, `medium`) and, separately, `high`, `BOUNDARY_API_KEY=local`, full suite: no value from `conformance/secrets.json` in any recording (plaintext search of every file, plus a `baml query` dump of every table; a hit names its span type and field). The recording must be sealed: the TypeScript and Go suites call the SDK's shutdown, the Python and Rust runtimes shut down at exit | Pass, all four languages, at both levels. Without `http_bodies = false` it fails at both: the Manager token, the setup-link token and a presigned S3 signature are in network spans' response bodies ([B2](gaps.md#b2)) |
| T4 | The end user's default: `ALIEN_TELEMETRY` unset, no Boundary API key, no saved login, full suite: recordings are written, all under `$BAML_HOME/btel/`, and no connection is made to `BOUNDARY_API_URL` (the sink) | Pass, all four languages |
| T5 | Isolation: with no API key at `medium`, recordings appear only under the isolated `BAML_HOME` (nothing new under a temporary `HOME`), and a fake login saved under that `HOME`'s `~/.baml` is ignored: no connection is made. A fake login planted in `BAML_HOME` is used (the sink sees a heartbeat) | Pass, all four languages |
| T6 | Telemetry never prints: the probe with `ALIEN_TELEMETRY` unset and no key (local files), the same with a fake key and an unreachable endpoint (a closed 127.0.0.1 port), and the T2 control run (the sink rejects every request with HTTP 503) exit 0 and print nothing on stderr that the probe does not also print with telemetry off (T1). The unreachable runs take about 5 s each, so the four languages run at once | Pass, all four languages |
