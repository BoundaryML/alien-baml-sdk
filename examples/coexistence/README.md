# Two BAML SDKs in one process (B4 repro)

`run.sh` installs the Alien SDK and the SDK of a second, unrelated BAML project
(`other/`) into one TypeScript application and one Python environment, calls
both, and asserts what happens. Run it after `scripts/generate.sh`:

```sh
examples/coexistence/run.sh
```

`other/` has one function, `greet`, and one function whose fully qualified
name (`setup.cloudformation_launch_url`) matches one of the Alien SDK's. With a
[local BAML build](../../README.md#building-against-an-unreleased-baml), every
install uses that build's runtime packages instead of the published ones.

`other/` keeps BAML's default package name, `baml_sdk`, as most BAML projects
do; the Alien SDK names its own (`package_name` in `baml/baml.toml`). In
Python, `run.sh` builds the `alien-sdk` wheel (it must hold only `alien_sdk/`),
installs both SDKs in either order, checks that every installed file of both
packages is intact, and uninstalls each again.

Installing both works. Running both does not: the last SDK to start the BAML
runtime owns the process, and calls through the other SDK fail with
`Function not found` or silently run the same-named function of the wrong
project. The full results and the one tested workaround are in
[docs/gaps.md#b4](../../docs/gaps.md#b4). `scripts/verify.sh`
runs this script, so a change in BAML's behavior shows up there.
