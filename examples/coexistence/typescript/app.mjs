// B4 repro: two BAML-generated SDKs in one Node process.
// Usage: node app.mjs alien-first | other-first
// Prints one JSON line per call so run.sh can assert the observed behavior.
const order = process.argv[2]
if (order !== "alien-first" && order !== "other-first") throw new Error("usage: node app.mjs alien-first|other-first")

const input = { templateUrl: "https://example.com/setup.yaml", region: "us-east-1", stackName: "alien-setup", parameters: {} }
const [first, second] = order === "alien-first" ? ["@alien-baml/sdk", "other-baml-sdk"] : ["other-baml-sdk", "@alien-baml/sdk"]
const modules = { [first]: await import(first) }
modules[second] = await import(second)
const alien = modules["@alien-baml/sdk"]
const other = modules["other-baml-sdk"]

function attempt(name, fn) {
  try {
    console.log(JSON.stringify({ call: name, ok: fn() }))
  } catch (error) {
    console.log(JSON.stringify({ call: name, error: `${error?.constructor?.name}: ${String(error?.message ?? error).split("\n")[0]}` }))
  }
}

attempt("alien.cloudFormationLaunchUrl", () => alien.cloudFormationLaunchUrl(input))
attempt("other.greet", () => other.greet("Alien"))
