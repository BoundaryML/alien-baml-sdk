import { homedir } from "node:os"
import { resolve } from "node:path"
import { defineConfig } from "vitest/config"

// The BAML runtime keeps state under BAML_HOME, and under ~/.baml when it is
// unset. Tests must never touch the developer's ~/.baml: refuse to start.
const bamlHome = process.env.BAML_HOME
if (!bamlHome || resolve(bamlHome) === resolve(homedir(), ".baml")) {
  throw new Error(
    "BAML_HOME must name an isolated directory (not ~/.baml) before the tests load BAML.\n" +
      "Run every suite with scripts/verify.sh, or this one by hand with a fresh home:\n" +
      '  BAML_HOME="$(mktemp -d)" pnpm -C sdk/typescript test\n' +
      "(conformance/README.md, \"Running\")",
  )
}

export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    // One stub server per file, scenarios in order.
    fileParallelism: false,
    sequence: { concurrent: false },
    testTimeout: 30_000,
    // Client defaults the tests set themselves; empty counts as unset.
    env: { ALIEN_API_KEY: "", ALIEN_WORKSPACE: "" },
  },
})
