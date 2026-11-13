// Normalization and comparison shared by every language harness.
//
// Normalized result (conformance/README.md):
// - JSON with sorted object keys;
// - object members whose value is null are dropped, so "absent" and "null"
//   compare equal across host languages (the BAML-level distinction is tested
//   by the ns_compat presence functions, which return a string);
// - errors are reduced to {code, retryable, http_status}, plus each of
//   `context`, `hint`, `request_id`, `internal` and `source` that the
//   scenario's expected error lists. A listed `source` is compared whole,
//   level by level: every field of each level, `message` included, and a
//   field the scenario leaves out must be null.
// - a `context` is JSON data and is compared as it is, nothing dropped: a
//   null member is not an absent one, `1` is not `"1"`, and an array keeps
//   its order. Only the order of an object's keys does not count, and a
//   context that is null is the same as none. Numbers are compared as
//   JavaScript reads them, so `1` and `1.0` are the same number.

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

// What an error has besides code, retryable, http_status, `context` and
// `source`.
const LISTED_FIELDS = ["hint", "request_id", "internal"]

// A context as it is compared: the JSON value itself, with no member dropped.
// isDeepStrictEqual already ignores the order of an object's keys.
function wholeContext(error) {
  return error.context ?? null
}

// One level of a `source` chain, and the levels below it, with every field:
// a harness cannot pass by leaving one out, nor a scenario by not listing it.
function wholeError(error) {
  if (error === null || error === undefined) return null
  if (typeof error !== "object" || Array.isArray(error)) return { not_an_error: error }
  const out = { code: error.code, message: error.message, retryable: error.retryable, http_status: error.http_status ?? null }
  for (const key of LISTED_FIELDS) out[key] = error[key] ?? null
  return { ...normalize(out), context: wholeContext(error), source: wholeError(error.source) }
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
    for (const key of LISTED_FIELDS) {
      if (key in wantError) {
        want[key] = wantError[key] ?? null
        got[key] = outcome.error[key] ?? null
      }
    }
    if ("context" in wantError) {
      want.context = wholeContext(wantError)
      got.context = wholeContext(outcome.error)
    }
    if ("source" in wantError) {
      want.source = wholeError(wantError.source)
      got.source = wholeError(outcome.error.source)
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
