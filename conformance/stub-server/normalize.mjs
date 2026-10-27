// Normalization and comparison shared by every language harness.
//
// Normalized result (conformance/README.md):
// - JSON with sorted object keys;
// - object members whose value is null are dropped, so "absent" and "null"
//   compare equal across host languages (the BAML-level distinction is tested
//   by the ns_compat presence functions, which return a string);
// - errors are reduced to {code, retryable, http_status}, plus `context` when
//   the scenario's expected error lists one.

import { isDeepStrictEqual } from "node:util"

export function normalize(value) {
  if (Array.isArray(value)) return value.map(normalize)
  if (value !== null && typeof value === "object") {
    const out = {}
    for (const key of Object.keys(value).sort()) {
      if (value[key] === null || value[key] === undefined) continue
      out[key] = normalize(value[key])
    }
    return out
  }
  return value
}

export function reduceError(error) {
  return normalize({
    code: error.code,
    retryable: error.retryable,
    http_status: error.http_status ?? null,
  })
}

export function compareOutcome(scenario, outcome) {
  const problems = []
  const wantError = scenario.expect_error ?? null
  if (wantError !== null) {
    if (!outcome.error) {
      problems.push(`expected error ${JSON.stringify(wantError)}, got success ${JSON.stringify(normalize(outcome.ok))}`)
      return problems
    }
    const want = reduceError(wantError)
    const got = reduceError(outcome.error)
    if ("context" in wantError) {
      want.context = normalize(wantError.context)
      got.context = normalize(outcome.error.context ?? null)
    }
    if (!isDeepStrictEqual(want, got)) {
      problems.push(`error: expected ${JSON.stringify(want)}, got ${JSON.stringify(got)} (message: ${JSON.stringify(outcome.error.message)})`)
    }
    return problems
  }
  if (outcome.error) {
    problems.push(`expected success, got error ${JSON.stringify(outcome.error)}`)
    return problems
  }
  if (!("ok" in outcome)) {
    problems.push(`outcome has neither ok nor error: ${JSON.stringify(outcome)}`)
    return problems
  }
  const expected = scenario.expect_result ?? {}
  if (!("ok" in expected)) {
    problems.push("scenario has neither expect_error nor expect_result.ok")
    return problems
  }
  const want = normalize(expected.ok)
  const got = normalize(outcome.ok)
  if (!isDeepStrictEqual(want, got)) {
    problems.push(`result: expected ${JSON.stringify(want)}, got ${JSON.stringify(got)}`)
  }
  return problems
}
