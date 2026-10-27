#!/usr/bin/env python3
"""Generates conformance scenarios for every imported operation into
conformance/scenarios/ops.*.json, and the Rust harness's table of those
operations into sdk/rust/tests/support/operations.rs (conformance/README.md).

    python3 scripts/operation_scenarios.py           rewrite them
    python3 scripts/operation_scenarios.py --check   fail if they differ

The specs are the oracle: each operation's method, path, query string, body
and success response come from the OpenAPI document (with the compatibility
overlay merged, as the importer does). The imported BAML (baml_src/ns_platform,
ns_manager) is read for which operations were imported, the function each
became and its parameter list, which the harnesses need to call it. Values are
synthesized from the schemas: every required property, plus optional ones near
the top, using the spec's `example` where a scalar has one. A string matches
its schema's `pattern` (`fit`): Alien's Rust client checks patterns, and the
spec's own ID examples are four characters short of theirs.

Every operation gets:
- `ops.<fn>.success`: every argument set; checks method, path, query, auth
  header and body, and decodes a spec-shaped response.
- `ops.<fn>.required_only` (when it has optional arguments): none of them set.
- `ops.<fn>.api_error`: the service's error body (Platform `APIError`, 404;
  Manager `ErrorResponse`, 404) becomes an AlienError with its code.
EXTRA_CASES below adds other error mapping cases for representative operations,
and UNION_CASES members of a union tagged by two fields that this SDK does not
know.
"""

from __future__ import annotations

import argparse
import json
import re
import re._parser as regex_syntax  # the standard library's regex parser (Python 3.11+)
import sys
from pathlib import Path
from typing import NoReturn

REPO = Path(__file__).resolve().parents[1]
SCENARIOS = REPO / "conformance" / "scenarios"
RUST_TABLE = REPO / "sdk" / "rust" / "tests" / "support" / "operations.rs"
CONFIG = REPO / "tools" / "openapi2baml" / "config.json"
PREFIX = "ops."
PHASE = 2
WORKSPACE = "my-workspace"
# Optional properties are filled in above this depth (0 = top level).
OPTIONAL_DEPTH = 1

SERVICES = {
    "platform": {
        "client": {"base_url": "${stub.base}", "api_key": "${secrets.apiKey}", "workspace": WORKSPACE},
        "auth": "Bearer ${secrets.apiKey}",
    },
    "manager": {
        "client": {"base_url": "${stub.base}", "token": "${secrets.managerToken}"},
        "auth": "Bearer ${secrets.managerToken}",
    },
}

API_ERROR = {"code": "RESOURCE_NOT_FOUND", "message": "No such resource.", "retryable": False,
             "internal": False, "requestId": "req_fixture_1", "hint": "Check the ID."}

# `ops.<fn>.api_error`, per service: the error body and the expected error,
# sent with the status the operation declares first (404 when it declares it
# or none). The bodies are valid for every error schema the specs use:
# Platform `APIError`, Manager `ErrorResponse` and `AlienError`.
API_ERROR_CASE = {
    "platform": (API_ERROR,
                 {"code": "RESOURCE_NOT_FOUND", "retryable": False,
                  "context": {"path": "path", "requestId": "req_fixture_1", "hint": "Check the ID."}}),
    "manager": ({"code": "NOT_FOUND", "message": "No such resource.", "details": "unknown id", "retryable": False, "internal": False},
                {"code": "NOT_FOUND", "retryable": False, "context": {"path": "path", "details": "unknown id"}}),
}

# Error mapping (docs/errors.md) on representative operations, plus a `null`
# body for a nullable response: (operation, case, response, expected error or
# {"ok": value}), every argument set. `path` in an expected context stands for
# the request path with its query string.
EXTRA_CASES = [
    ("platform.managers.get", "api_error_404",
     {"status": 404, "json": API_ERROR},
     {"code": "RESOURCE_NOT_FOUND", "retryable": False, "http_status": 404,
      "context": {"path": "path", "requestId": "req_fixture_1", "hint": "Check the ID."}}),
    ("platform.deployments.create", "api_error_retryable_409",
     {"status": 409, "json": {"code": "CONFLICT", "message": "Busy.", "retryable": True, "internal": False, "context": {"field": "name"}}},
     {"code": "CONFLICT", "retryable": True, "http_status": 409,
      "context": {"path": "path", "details": "{\"field\":\"name\"}"}}),
    ("platform.deployments.list", "server_error_500_text",
     {"status": 500, "text": "upstream failed"},
     {"code": "PLATFORM_HTTP_ERROR", "retryable": True, "http_status": 500,
      "context": {"path": "path", "body": "upstream failed"}}),
    ("platform.apiKeys.revoke", "forbidden_403",
     {"status": 403, "json": {"code": "FORBIDDEN", "message": "Not allowed.", "internal": False}},
     {"code": "FORBIDDEN", "retryable": False, "http_status": 403, "context": {"path": "path"}}),
    ("platform.auth.whoami", "unauthorized_401_empty",
     {"status": 401},
     {"code": "PLATFORM_HTTP_ERROR", "retryable": False, "http_status": 401, "context": {"path": "path", "body": ""}}),
    ("platform.projects.delete", "rate_limited_429",
     {"status": 429, "text": "slow down"},
     {"code": "PLATFORM_HTTP_ERROR", "retryable": True, "http_status": 429,
      "context": {"path": "path", "body": "slow down"}}),
    ("platform.releases.get", "malformed_200",
     {"status": 200, "json": {"unexpected": True}},
     {"code": "MALFORMED_RESPONSE", "retryable": False, "http_status": 200,
      "context": {"path": "path", "decode_error": "missing required field `id`"}}),
    ("platform.deployments.getUpdateOperation", "null_body",
     {"status": 200, "json": None},
     {"ok": None}),
    ("manager.commands.createCommand", "error_body_400",
     {"status": 400, "json": {"code": "INVALID_COMMAND", "message": "Bad command.", "details": "command is required"}},
     {"code": "INVALID_COMMAND", "retryable": False, "http_status": 400,
      "context": {"path": "path", "details": "command is required"}}),
    ("manager.commands.getCommandStatus", "http_error_502_text",
     {"status": 502, "text": "bad gateway"},
     {"code": "MANAGER_HTTP_ERROR", "retryable": False, "http_status": 502,
      "context": {"path": "path", "body": "bad gateway"}}),
    ("manager.commands.getCommandStatus", "network_failure",
     {"drop": True},
     {"code": "COMMUNICATION_ERROR", "retryable": True, "http_status": None,
      "context": {"method": "GET", "path": "path"}}),
]

# A union whose members are told apart by two fields (`ExternalBinding`:
# `type`, then `service` where members share a `type`), inside a success
# response: (operation, case, where the union's values go in the response,
# their wire values, and the expected host values or error). A `type`, or a
# `service` of a known `type`, that no member has decodes as the union's one
# catch-all member, `{"type": "UNKNOWN", "raw": <as sent>}`; a known pair that
# does not decode fails the call (docs/api-shape.md, "Unions").
S3_BINDING = {"type": "storage", "service": "s3", "bucketName": "files"}
NEW_SERVICE = {"type": "storage", "service": "minio", "endpoint": "http://minio.internal:9000", "bucketName": "media"}
NEW_TYPE = {"type": "search_index", "service": "opensearch", "indexName": "docs"}
UNION_CASES = [
    ("platform.deployments.get", "external_binding_new_service", ["stackSettings", "externalBindings"],
     {"files": S3_BINDING, "media": NEW_SERVICE},
     {"ok": {"files": S3_BINDING, "media": {"type": "UNKNOWN", "raw": NEW_SERVICE}}}),
    ("platform.deployments.get", "external_binding_new_type", ["stackSettings", "externalBindings"],
     {"docs": NEW_TYPE},
     {"ok": {"docs": {"type": "UNKNOWN", "raw": NEW_TYPE}}}),
    ("platform.deployments.get", "external_binding_known_malformed", ["stackSettings", "externalBindings"],
     {"files": {**S3_BINDING, "forcePathStyle": "yes"}},
     # `json_path` is relative to the binding (docs/gaps.md, B21).
     {"code": "MALFORMED_RESPONSE", "retryable": False, "http_status": 200,
      "context": {"path": "path", "decode_error": "expected boolean", "json_path": ".forcePathStyle"}}),
]


def fail(message: str) -> NoReturn:
    raise SystemExit(f"operation_scenarios: {message}")


# ---------------------------------------------------------------------------
# Specs


def load_spec(path: str, overlays: list[str]) -> dict:
    doc = json.loads((REPO / path).read_text(encoding="utf-8"))
    for overlay in overlays:
        for action in json.loads((REPO / overlay).read_text(encoding="utf-8"))["actions"]:
            node = doc
            for segment in re.findall(r"\['([^']*)'\]", action["target"]):
                node = node[segment]
            add_absent(node, action["update"])
    return doc


def add_absent(target: dict, update: dict) -> None:
    """The importer's overlay rule: add what is absent, recursively."""
    for key, value in update.items():
        if key in target and isinstance(target[key], dict) and isinstance(value, dict):
            add_absent(target[key], value)
        else:
            target.setdefault(key, value)


class Spec:
    def __init__(self, doc: dict) -> None:
        self.doc = doc
        self.operations = {}
        for path, item in doc["paths"].items():
            for method, op in item.items():
                if isinstance(op, dict) and "operationId" in op:
                    self.operations[op["operationId"]] = (method.upper(), path, op, item)

    def deref(self, value: dict) -> dict:
        while "$ref" in value:
            node = self.doc
            for segment in value["$ref"].lstrip("#/").split("/"):
                node = node[segment.replace("~1", "/").replace("~0", "~")]
            value = node
        return value

    def parameters(self, op: dict, item: dict) -> list[dict]:
        """Path-item parameters, overridden by the operation's, in spec order."""
        merged: dict[tuple[str, str], dict] = {}
        for raw in item.get("parameters", []) + op.get("parameters", []):
            param = self.deref(raw)
            merged[(param["name"], param["in"])] = param
        return list(merged.values())


# ---------------------------------------------------------------------------
# Values from schemas


def is_null(schema: dict) -> bool:
    return schema.get("type") == "null" or schema.get("enum") == [None]


def schema_type(schema: dict) -> str | None:
    kind = schema.get("type")
    if isinstance(kind, list):
        kind = next((k for k in kind if k != "null"), None)
    return kind


class Values:
    def __init__(self, spec: Spec) -> None:
        self.spec = spec

    def merge_all_of(self, schema: dict) -> dict:
        merged: dict = {"type": "object", "properties": {}, "required": []}
        for raw in schema["allOf"]:
            part = self.spec.deref(raw)
            if "allOf" in part:
                part = self.merge_all_of(part)
            for key in ("oneOf", "anyOf"):
                if key in part:
                    part = self.merge_all_of({"allOf": [self.first_member(part, key)[0]]})
            merged["properties"].update(part.get("properties", {}))
            merged["required"] += [r for r in part.get("required", []) if r not in merged["required"]]
            for key in ("additionalProperties", "enum", "const", "items", "format", "pattern"):
                if key in part:
                    merged[key] = part[key]
            if part.get("type") not in (None, "object"):
                merged["type"] = part["type"]
        if not merged["properties"]:
            del merged["properties"]
        return merged

    def first_member(self, schema: dict, key: str) -> tuple[dict, dict]:
        """The first non-null member of a union, and the properties its
        discriminator pins (the importer makes them the member's tag)."""
        members = [m for m in schema[key] if not is_null(self.spec.deref(m))]
        if not members:
            fail(f"union without a non-null member: {schema}")
        chosen = members[0]
        pinned = {}
        discriminator = schema.get("discriminator")
        if discriminator:
            prop = discriminator["propertyName"]
            for value, ref in discriminator.get("mapping", {}).items():
                if chosen.get("$ref") == ref:
                    pinned[prop] = value
        return chosen, pinned

    def value(self, raw: dict, name: str, depth: int, request: bool, seen: tuple = ()) -> tuple[object, object]:
        """A value for `raw`, as (wire JSON, host view). The host view is what
        a generated SDK returns: BAML field names (B14), and keys a class does
        not list under its `additionalProperties` field. Requests use neither,
        so their two forms are the same."""
        ref = raw.get("$ref")
        if ref in seen:
            fail(f"recursive required schema at {ref}")
        seen = seen + ((ref,) if ref else ())
        schema = self.spec.deref(raw)
        if "allOf" in schema:
            schema = self.merge_all_of(schema)
        for key in ("oneOf", "anyOf"):
            if key in schema:
                member, pinned = self.first_member(schema, key)
                wire, host = self.value(member, name, depth, request, seen)
                if isinstance(wire, dict):
                    wire.update(pinned)
                    host.update({field_name(k): v for k, v in pinned.items()})
                return wire, host
        if "const" in schema:
            return schema["const"], schema["const"]
        example = scalar_example(schema)
        if example is not None:
            example = fit(schema, example)
            return example, example
        if "enum" in schema:
            first = next(v for v in schema["enum"] if v is not None)
            return first, first
        kind = schema_type(schema)
        if kind is None and ("properties" in schema or "additionalProperties" in schema):
            kind = "object"
        if kind is None and "items" in schema:
            kind = "array"
        if kind in SCALARS:
            scalar = fit(schema, SCALARS[kind](schema, name))
            return scalar, scalar
        if kind == "array":
            wire, host = self.value(schema.get("items", {}), name + "-item", depth + 1, request, seen)
            return [wire], [host]
        if kind != "object":
            return {"key": "value"}, {"key": "value"}  # untyped: BAML json
        wire, host = {}, {}
        required = set(schema.get("required", []))
        for prop, prop_schema in schema.get("properties", {}).items():
            optional = prop not in required
            if optional and (depth >= OPTIONAL_DEPTH or prop_schema.get("$ref") in seen):
                continue
            if request and field_name(prop) != prop:
                if optional:
                    continue
                fail(f"a request needs the renamed field {prop!r}; requests here use one form for every language")
            wire[prop], host[field_name(prop)] = self.value(prop_schema, prop, depth + 1, request, seen)
        extra = schema.get("additionalProperties")
        if extra is True:
            extra = {}
        if isinstance(extra, dict) and not schema.get("properties"):
            key = name + "-key"
            wire[key], host[key] = self.value(extra, name, depth + 1, request, seen)  # a map
        elif isinstance(extra, dict) and not request:
            # A class that keeps unlisted keys (DESIGN.md): one such key.
            extra_wire, extra_host = self.value(extra, "extra", depth + 1, request, seen)
            wire["extra-property"] = extra_wire
            host["additionalProperties"] = {"extra-property": extra_host}
        elif not schema.get("properties") and not wire:
            wire, host = {"key": "value"}, {"key": "value"}  # free-form JSON
        return wire, host


def scalar_example(schema: dict) -> object:
    """The schema's `example`, when it is a value of the schema's scalar type
    (and one of its enum values, if it has any); else None."""
    if "example" not in schema:
        return None
    example = schema["example"]
    kind = schema_type(schema) or ("string" if isinstance(example, str) else None)
    fits = {"string": isinstance(example, str), "boolean": isinstance(example, bool),
            "integer": isinstance(example, int) and not isinstance(example, bool),
            "number": isinstance(example, (int, float)) and not isinstance(example, bool)}
    if not fits.get(kind, False):
        return None
    if "enum" in schema and example not in schema["enum"]:
        return None
    return example


SCALARS = {
    "string": lambda schema, name: {"date-time": "2026-10-01T00:00:00Z", "uri": "https://example.com/" + name,
                                    "email": "user@example.com"}.get(schema.get("format"), f"{name}-value"),
    "integer": lambda schema, name: max(schema.get("minimum", 2), min(schema.get("maximum", 2), 2)),
    "number": lambda schema, name: 1.5,
    "boolean": lambda schema, name: True,
}


def fit(schema: dict, value: object) -> object:
    """`value`, made to satisfy the string constraints Alien's Rust client
    checks when it is a string that does not: `format: uuid` (a fixed UUID),
    `pattern` (`matching`), and `minLength` and `maxLength` (repeated or cut
    short when there is no pattern)."""
    if not isinstance(value, str):
        return value
    low, high, pattern = schema.get("minLength", 0), schema.get("maxLength"), schema.get("pattern")
    if schema.get("format") == "uuid" and not re.fullmatch(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", value):
        value = UUID
    if pattern and not (re.search(pattern, value) and low <= len(value)):
        value = matching(pattern, value, low)
    elif not pattern and len(value) < low:
        value = (value * low)[:low]
    if high is not None and len(value) > high:
        if pattern:
            fail(f"pattern {pattern!r}: {value!r} is longer than {high}")
        value = value[:high]
    return value


UUID = "123e4567-e89b-42d3-a456-426614174000"


def matching(pattern: str, seed: str, min_length: int = 0) -> str:
    """A short string `pattern` matches that follows `seed`: literals are
    written as they are, each character class takes the next character of
    `seed` it allows (else its first), every repetition repeats as few times
    as it may (more while the string is shorter than `min_length`), and an
    alternation takes its first branch that writes something.
    `dep_[0-9a-z]{28}$` turns `dep_0c29fq4a2yjb7kx3smwdgxlc` (the spec's
    example) into `dep_0c29fq4a2yjb7kx3smwdgxlcdep0`. Lookaheads are not
    followed, so the result is checked, and a pattern it does not match fails
    generation."""
    out: list[str] = []
    at = 0
    fallback = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-_.:/"

    def allows(items: list, char: str) -> bool:
        negate = bool(items) and items[0][0] == regex_syntax.NEGATE
        hit = any(
            (op == regex_syntax.LITERAL and ord(char) == arg)
            or (op == regex_syntax.RANGE and arg[0] <= ord(char) <= arg[1])
            or (op == regex_syntax.CATEGORY and re.fullmatch(CATEGORIES[arg], char) is not None)
            for op, arg in items[negate:])
        return hit != negate

    def pick(items: list) -> str:
        nonlocal at
        for step in range(len(seed)):
            char = seed[(at + step) % len(seed)]
            if allows(items, char):
                at = (at + step + 1) % len(seed)
                return char
        return next(c for c in fallback if allows(items, c))

    def write(nodes) -> None:
        nonlocal at
        for op, arg in nodes:
            if op == regex_syntax.LITERAL:
                if seed and seed[at % len(seed)] == chr(arg):
                    at = (at + 1) % len(seed)
                out.append(chr(arg))
            elif op == regex_syntax.IN:
                out.append(pick(arg))
            elif op == regex_syntax.NOT_LITERAL:
                out.append(pick([(regex_syntax.NEGATE, None), (regex_syntax.LITERAL, arg)]))
            elif op == regex_syntax.ANY:
                out.append(pick([(regex_syntax.NEGATE, None), (regex_syntax.LITERAL, ord("\n"))]))
            elif op == regex_syntax.CATEGORY:
                out.append(pick([(op, arg)]))
            elif op in (regex_syntax.MAX_REPEAT, regex_syntax.MIN_REPEAT):
                low, high, body = arg
                count = 0
                while count < low or (len("".join(out)) < min_length and count < high):
                    before = len("".join(out))
                    write(body)
                    count += 1
                    if count >= low and len("".join(out)) == before:
                        break
            elif op == regex_syntax.BRANCH:
                mark = len(out)
                for branch in arg[1]:
                    write(branch)
                    if "".join(out[mark:]):
                        break
            elif op == regex_syntax.SUBPATTERN:
                write(arg[3])
            elif op not in (regex_syntax.AT, regex_syntax.ASSERT, regex_syntax.ASSERT_NOT):
                fail(f"pattern {pattern!r}: cannot write a match for {op}")

    write(regex_syntax.parse(pattern))
    value = "".join(out)
    if not re.search(pattern, value) or len(value) < min_length:
        fail(f"pattern {pattern!r}: wrote {value!r}, which does not match it or is shorter than {min_length}")
    return value


# The categories (`\d`, `\s`, `\w`) a character class can name.
CATEGORIES = {
    regex_syntax.CATEGORY_DIGIT: r"\d", regex_syntax.CATEGORY_NOT_DIGIT: r"\D",
    regex_syntax.CATEGORY_SPACE: r"\s", regex_syntax.CATEGORY_NOT_SPACE: r"\S",
    regex_syntax.CATEGORY_WORD: r"\w", regex_syntax.CATEGORY_NOT_WORD: r"\W",
}

# tools/openapi2baml/src/naming.rs: field names BAML reserves.
RESERVED_FIELD_NAMES = {
    "await", "break", "catch", "catch_all", "class", "continue", "defer", "else", "enum", "for",
    "function", "generator", "if", "implement", "implements", "in", "instanceof", "is", "let",
    "match", "retry_policy", "return", "spawn", "template_string", "test", "testset", "throw",
    "throws", "while",
}


# tools/openapi2baml/src/naming.rs: argument names BAML reserves, plus `path`.
RESERVED_PARAM_NAMES = {
    "await", "baml", "break", "catch", "catch_all", "class", "client", "continue", "defer", "else",
    "enum", "extends", "false", "for", "function", "generator", "if", "implement", "implements",
    "in", "instanceof", "interface", "is", "let", "match", "null", "path", "requires", "retry_policy",
    "return", "root", "self", "spawn", "template_string", "test", "testset", "throw", "throws", "true",
    "while",
}


def identifier(wire: str, reserved: set[str]) -> str:
    """tools/openapi2baml/src/naming.rs `identifier`."""
    name = re.sub(r"[^A-Za-z0-9_]", "_", wire)
    if not name or name[:1].isdigit():
        name = "_" + name
    return name + "_" if name in reserved else name


def sdk_name(wire: str) -> str:
    """tools/openapi2baml/src/naming.rs `sdk_name`: camelCased when the wire
    name has separators (`custom_domains` → `customDomains`), else kept."""
    if not wire.isascii() or not wire.isalnum():
        words = [w for w in re.split(r"[^A-Za-z0-9]+", wire) if w]
        wire = "".join(w[:1].upper() + w[1:] for w in words)
        if wire[:1].isdigit():
            wire = "_" + wire
        wire = wire[:1].lower() + wire[1:]
    return wire


def field_name(wire: str) -> str:
    """tools/openapi2baml/src/naming.rs `field_name`: the BAML field name of a
    wire name (`custom_domains` → `customDomains`, `test` → `test_`, B14)."""
    return identifier(sdk_name(wire), RESERVED_FIELD_NAMES)


def param_name(wire: str) -> str:
    """tools/openapi2baml/src/naming.rs `argument_name`: the BAML argument
    name of a parameter's wire name (`command_id` → `commandId`, `If-Match` →
    `ifMatch`, `path` → `path_`)."""
    return identifier(sdk_name(wire), RESERVED_PARAM_NAMES)


# ---------------------------------------------------------------------------
# Imported BAML: which function each operation became, and its parameters


FUNCTION = re.compile(r"\(operation `([^`]+)`\)\.(?: Deprecated\.)?\n(?:///.*\n)*function (\w+)\((.*)\) -> (.+?) throws ")


def split_params(text: str) -> list[str]:
    parts, depth, current = [], 0, ""
    for ch in text:
        depth += ch in "<([" and 1 or ch in ">)]" and -1 or 0
        if ch == "," and depth == 0:
            parts.append(current.strip())
            current = ""
        else:
            current += ch
    if current.strip():
        parts.append(current.strip())
    return parts


def baml_functions(service: str) -> dict[str, dict]:
    """Every imported operation, by operation ID: its BAML path (`platform.managers.get`,
    or `platform.acceptWorkspaceInvitation` without a group) and parameters."""
    out = {}
    root = REPO / "baml" / "baml_src" / f"ns_{service}"
    for path in sorted([root / "operations.baml", *root.glob("ns_*/operations.baml")]):
        if not path.exists():
            continue
        prefix = service if path.parent == root else f"{service}.{path.parent.name[len('ns_'):]}"
        for op_id, name, params, _ in FUNCTION.findall(path.read_text(encoding="utf-8")):
            parsed = []
            for param in split_params(params)[1:]:  # the client comes first
                match = re.fullmatch(r"(\w+): (.+?)( = null)?", param)
                if not match:
                    fail(f"{path}: cannot read parameter {param!r}")
                kind = match.group(2).replace(f"root.{service}.", "")
                parsed.append({"name": match.group(1), "type": kind.rstrip("?"), "optional": bool(match.group(3))})
            if op_id in out:
                fail(f"{path}: operation {op_id} is imported twice")
            out[op_id] = {"fn": f"{prefix}.{name}", "params": parsed}
    return out


# ---------------------------------------------------------------------------
# Scenarios


# `${` in a value from the spec (`projects/${projectName}`) that is not one of
# the stub's templates (conformance/README.md).
NOT_A_TEMPLATE = re.compile(r"\$\{(?!stub\.base\}|secrets\.|now[}+-]|\$\})")


def literal(value: object) -> object:
    """`value` with every `${` that is not a template written `${$}{`."""
    if isinstance(value, str):
        return NOT_A_TEMPLATE.sub("${$}{", value)
    if isinstance(value, list):
        return [literal(v) for v in value]
    if isinstance(value, dict):
        return {k: literal(v) for k, v in value.items()}
    return value


def percent_encode(text: str) -> str:
    return "".join(c if re.fullmatch(r"[A-Za-z0-9\-._~]", c) else "".join(f"%{b:02X}" for b in c.encode()) for c in text)


def parameter_value(spec: Spec, param: dict) -> object:
    """A path or query argument: a list gets two items (form style repeats the
    key) and a query string a space (percent-encoded)."""
    schema = spec.deref(param["schema"])
    if "allOf" in schema:
        schema = Values(spec).merge_all_of(schema)
    name = param["name"]
    for source in (param, schema):
        example = scalar_example({**schema, "example": source["example"]}) if "example" in source else None
        if example is not None:
            return fit(schema, example)
    if schema_type(schema) == "array":
        items = spec.deref(schema.get("items", {}))
        return [v for v in items["enum"] if v is not None][:2] if "enum" in items else [fit(items, f"{name}-a"), fit(items, f"{name}-b")]
    if "enum" in schema:
        return next(v for v in schema["enum"] if v is not None)
    if schema_type(schema) in ("integer", "number"):
        # A whole number for a `number` too: a client writes it as `2`, not `2.0`.
        return SCALARS["integer"](schema, name)
    if schema_type(schema) != "string":
        fail(f"parameter {name}: unsupported schema {schema}")
    if "format" in schema:
        return fit(schema, SCALARS["string"](schema, name))
    return fit(schema, f"{name} value" if param["in"] == "query" else f"{name}-value")


def operation_scenarios(service: str, op_id: str, spec: Spec, values: Values, fn: dict) -> dict[str, dict]:
    method, template, op, item = spec.operations[op_id]
    # By BAML argument name: the wire name, camelCased when it has separators
    # (`If-Match` → `ifMatch`), and a reserved word gets a trailing `_`.
    params = {param_name(p["name"]): p for p in spec.parameters(op, item)}
    body_schema = None
    if "requestBody" in op:
        body_schema = spec.deref(op["requestBody"])["content"]["application/json"]["schema"]
    status, response = next((c, r) for c, r in sorted(op["responses"].items()) if c.startswith("2"))
    response = spec.deref(response)
    response_schema = (response.get("content") or {}).get("application/json", {}).get("schema")

    # Argument values, by BAML argument name.
    full: dict[str, object] = {}
    for param in fn["params"]:
        if param["name"] == "body":
            if body_schema is None:
                fail(f"{op_id}: BAML takes a body the spec does not have")
            full["body"] = values.value(body_schema, "body", 0, request=True)[0]
        else:
            spec_param = params.get(param["name"])
            if spec_param is None:
                fail(f"{op_id}: BAML parameter {param['name']} is not in the spec")
            full[param["name"]] = parameter_value(spec, spec_param)

    def request(args: dict) -> tuple[str, dict]:
        path = re.sub(r"\{([^}]+)\}", lambda m: percent_encode(str(args[param_name(m.group(1))])), template)
        query = []
        names = [p["name"] for p in params.values() if p["in"] == "query"]
        if "workspace" in names:  # the importer's rule: workspace first
            names = ["workspace"] + [n for n in names if n != "workspace"]
        for name in names:
            value = WORKSPACE if name == "workspace" and service == "platform" else args.get(param_name(name))
            if value is None:
                continue
            for item_value in value if isinstance(value, list) else [value]:
                query.append(f"{name}={percent_encode(str(item_value))}")
        if query:
            path += "?" + "&".join(query)
        expect: dict = {"method": method, "path": path,
                        "headers": {"authorization": SERVICES[service]["auth"], "accept": "application/json"}}
        for key, param in params.items():
            if param["in"] == "header" and key in args:
                expect["headers"][param["name"].lower()] = args[key]
        if "body" in args:
            expect["headers"]["content-type"] = "application/json"
            expect["json"] = args["body"]
        else:
            expect["headers_absent"] = ["content-type"]
            expect["body_empty"] = True
        return path, expect

    def scenario(case: str, args: dict, respond: dict, outcome: dict, notes: str) -> dict:
        path, expect = request(args)
        error = None if "ok" in outcome else dict(outcome)
        if error and "context" in error:
            error["context"] = {k: (path if v == "path" else v) for k, v in error["context"].items()}
        return literal({
            "id": f"{PREFIX}{fn['fn']}.{case}",
            "phase": PHASE,
            "notes": f"Generated by scripts/operation_scenarios.py from `{method} {template}` "
                     f"(operation `{op_id}`): {notes}",
            "call": {
                "op": fn["fn"],
                "params": fn["params"],
                "args": {"client": SERVICES[service]["client"], **args},
            },
            "exchanges": [{"expect": expect, "respond": respond}],
            "expect_result": {"ok": outcome["ok"]} if "ok" in outcome else None,
            "expect_error": error,
            "assert": [],
        })

    if response_schema is not None:
        wire, host = values.value(response_schema, "response", 0, request=False)
        success = ({"status": int(status), "json": wire}, {"ok": host})
    else:
        success = ({"status": int(status)}, {"ok": None})
    out = {}
    s = scenario("success", full, *success, "every argument set; a response with every required field.")
    out[s["id"]] = s
    required = {p["name"]: full[p["name"]] for p in fn["params"] if not p["optional"]}
    if len(required) < len(full):
        s = scenario("required_only", required, *success, "only the required arguments.")
        out[s["id"]] = s
    declared = sorted(c for c in op["responses"] if re.fullmatch(r"4\d\d", c))
    error_status = 404 if "404" in declared or not declared else int(declared[0])
    body, error = API_ERROR_CASE[service]
    s = scenario("api_error", full, {"status": error_status, "json": body}, {**error, "http_status": error_status},
                 f"the service's error body with HTTP {error_status}, every argument set.")
    out[s["id"]] = s
    for extra_fn, case, respond, outcome in EXTRA_CASES:
        if extra_fn == fn["fn"]:
            s = scenario(case, full, respond, outcome, "a case from EXTRA_CASES.")
            out[s["id"]] = s
    for union_fn, case, where, sent, outcome in UNION_CASES:
        if union_fn == fn["fn"]:
            respond, expected = json.loads(json.dumps(success))  # copies
            set_at(respond["json"], where, sent)
            if "ok" in outcome:
                set_at(expected["ok"], where, outcome["ok"])
                outcome = expected
            s = scenario(case, full, respond, outcome, "a case from UNION_CASES.")
            out[s["id"]] = s
    return out


def set_at(value: dict, where: list[str], to: object) -> None:
    """Sets `value[where[0]][where[1]]…` to `to`; every level but the last must exist."""
    for key in where[:-1]:
        if not isinstance(value.get(key), dict):
            fail(f"no object at {key!r} on the way to {'.'.join(where)}")
        value = value[key]
    value[where[-1]] = to


def generate() -> tuple[dict[str, str], str]:
    """The scenario files by name, and the Rust table."""
    config = json.loads(CONFIG.read_text(encoding="utf-8"))
    files: dict[str, str] = {}
    covered: dict[str, tuple[str, dict]] = {}
    for service in config["services"]:
        spec = Spec(load_spec(service["spec"], service.get("overlays", [])))
        values = Values(spec)
        functions = baml_functions(service["name"])
        # The importer's selection (tools/openapi2baml/DESIGN.md): every
        # operation but those marked x-speakeasy-ignore, or a list.
        selection = service["operations"]
        if selection == "all":
            selection = [op_id for op_id, (_, _, op, _) in spec.operations.items() if op.get("x-speakeasy-ignore") is not True]
        if sorted(selection) != sorted(functions):
            fail(f"{service['name']}: imported operations differ from the config: "
                 f"missing {sorted(set(selection) - set(functions))[:5]}, extra {sorted(set(functions) - set(selection))[:5]}")
        for op_id in sorted(functions):
            for scenario_id, scenario in operation_scenarios(service["name"], op_id, spec, values, functions[op_id]).items():
                files[f"{scenario_id}.json"] = json.dumps(scenario, indent=2, ensure_ascii=False) + "\n"
            covered[functions[op_id]["fn"]] = (service["name"], functions[op_id])
    missing = {fn for fn, *_ in EXTRA_CASES + UNION_CASES} - set(covered)
    if missing:
        fail(f"EXTRA_CASES or UNION_CASES name operations that are not imported: {sorted(missing)}")
    return files, rust_table(covered)


# ---------------------------------------------------------------------------
# The Rust harness's operation table


CLASS = re.compile(r"^class (\w+) ", re.M)
ALIAS = re.compile(r"^type (\w+) = (.+)$", re.M)


def rust_table(operations: dict[str, tuple[str, dict]]) -> str:
    """`sdk/rust/tests/support/operations.rs`: one `op!` row per operation
    (`sdk/rust/tests/conformance.rs` explains the rows). A required argument
    whose type is a class names it: the generated function takes it as
    `impl Borrow<X>`, which leaves the type to the caller."""
    classes: dict[str, set[str]] = {}
    for service in sorted({service for service, _ in operations.values()}):
        text = (REPO / "baml" / "baml_src" / f"ns_{service}" / "types.baml").read_text(encoding="utf-8")
        names = set(CLASS.findall(text))
        # An alias of a single class is that class.
        names |= {alias for alias, target in ALIAS.findall(text) if target.strip() in names}
        classes[service] = names
    rows = []
    for op in sorted(operations):
        service, fn = operations[op]
        *path, name = op.split(".")
        args = ["a", "::".join(path) + f"::{name}_async"]
        for param in fn["params"]:
            kind = "opt" if param["optional"] else "req"
            arg = f'{kind} "{param["name"]}"'
            if not param["optional"] and param["type"] in classes[service]:
                arg += f": {service}::{param['type']}"
            args.append(arg)
        rows.append(f'        "{op}" => op!({", ".join(args)}),\n')
    return (
        "// Generated by scripts/operation_scenarios.py from the imported BAML; do not edit.\n"
        "// Included by tests/conformance.rs, which defines `op!` and its helpers.\n"
        "\n"
        "async fn operation(op: &str, a: &Value) -> Result<Value, Error> {\n"
        "    match op {\n"
        + "".join(rows)
        + '        other => Err(harness(format!("no Rust binding for operation {other}"))),\n'
        "    }\n"
        "}\n"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="fail if the scenarios on disk differ")
    args = parser.parse_args()
    files, table = generate()
    existing = {p.name: p for p in SCENARIOS.glob(f"{PREFIX}*.json")}
    if args.check:
        stale = sorted(n for n in files if n not in existing or existing[n].read_text(encoding="utf-8") != files[n])
        extra = sorted(set(existing) - set(files))
        if not RUST_TABLE.exists() or RUST_TABLE.read_text(encoding="utf-8") != table:
            stale.append(str(RUST_TABLE.relative_to(REPO)))
        if stale or extra:
            print(f"operation scenarios are out of date (run scripts/operation_scenarios.py): "
                  f"{len(stale)} differ or are missing, {len(extra)} extra: {(stale + extra)[:5]}", file=sys.stderr)
            return 1
        print(f"operation scenarios: {len(files)} files and the Rust table up to date")
        return 0
    for name in set(existing) - set(files):
        existing[name].unlink()
    for name, text in files.items():
        (SCENARIOS / name).write_text(text, encoding="utf-8")
    RUST_TABLE.write_text(table, encoding="utf-8")
    print(f"operation scenarios: wrote {len(files)} files and the Rust table")
    return 0


if __name__ == "__main__":
    sys.exit(main())
