# Two BAML SDKs in one process (B4 repro)

`run.sh` installs the Alien SDK and the SDK of a second, unrelated BAML project
(`other/`) into one TypeScript application and one Python environment, calls
both, and asserts what happens. Run it after `scripts/generate.sh`:

```sh
examples/coexistence/run.sh
```

`other/` has one function, `greet`, and one function whose fully qualified
name (`setup.cloudformation_launch_url`) matches one of the Alien SDK's. With a
[local BAML build](../../README.md#testing-an-unreleased-baml-build), every install uses that
build's runtime packages instead of the published ones.

Today the last SDK to start the BAML runtime owns the process: calls through
the other SDK fail with `Function not found` or silently run the
same-named function of the wrong project. The full results and the one tested
workaround are in [docs/gaps.md#b4](../../docs/gaps.md#b4). `scripts/verify.sh`
runs this script, so a change in BAML's behavior shows up there.
