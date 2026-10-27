#!/usr/bin/env python3
"""Extract a naming oracle from Alien's Speakeasy-generated TypeScript SDKs.

For every operation in each OpenAPI spec, records how the incumbent SDK exposes
it (sub-client group, method name, argument shape, request/response types).
The exposure is read from the generated sources only: src/sdk/*.ts class
methods call src/funcs/<fn>.ts, and each funcs file carries
`operationID: "<id>"`. Spec x-speakeasy-* hints are used solely to report where
they would predict a different name than the SDK actually has.

It also records the SDK's name for every response field whose wire name it
renames or that has a character other than a letter or digit (`field_names`):
the keys of each model's `$inboundSchema` object and its `remap$` table. And
for every operation, the SDK's name for each of its parameters
(`param_names`, wire name to SDK name): the keys of the request type's
`$outboundSchema` object and its `remap$` table, restricted to the
operation's spec parameters (the other key is the body).

Usage:
    extract_naming_oracle.py --alien-ref /path/to/alien --out naming-oracle.json
"""

import argparse
import json
import os
import re
import subprocess
from pathlib import Path

SDKS = {
    "platform": "client-sdks/platform",
    "manager": "client-sdks/manager",
}
PREDICTION_RULES = {
    "platform": "group = x-speakeasy-group (absent: root); "
    "method = x-speakeasy-name-override (absent: camelCase(operationId))",
    "manager": "group = camelCase(tags[0]) (no tags: root); method = camelCase(operationId)",
}
HTTP_METHODS = ("get", "put", "post", "delete", "patch", "head", "options", "trace")

CLASS_RE = re.compile(r"^export class (\w+) extends ClientSDK \{", re.M)
GETTER_RE = re.compile(r"^  get (\w+)\(\): (\w+) \{", re.M)
FUNC_IMPORT_RE = re.compile(r'^import \{ (\w+) \} from "\.\./funcs/(\w+)\.js";', re.M)
METHOD_RE = re.compile(
    r"^  async (\w+)\(\n(.*?)\n  \): Promise<(.*?)> \{\n\s*return unwrapAsync\((\w+)\(",
    re.M | re.S,
)
PARAM_START_RE = re.compile(r"^    (\w+)(\??):(.*)$")
OPERATION_ID_RE = re.compile(r'operationID: "([^"]+)"')
HTTP_METHOD_RE = re.compile(r'^\s+method: "([A-Z]+)",', re.M)
PATH_RE = re.compile(r'pathToFunc\(\s*"([^"]+)"')
# The operation request type a func validates its input with (functions that
# take only a body validate a `models.` type instead).
REQUEST_SCHEMA_RE = re.compile(r"\boperations\.(\w+)\$outboundSchema(?:\s*\.optional\(\))?\s*\.parse\(")
# A model file's declarations: from one `export` to the next.
EXPORT_RE = re.compile(r"^export (?:(?!^export ).)*", re.M | re.S)
INBOUND_RE = re.compile(r"export const (\w+)\$inboundSchema\b")
OUTBOUND_RE = re.compile(r"export const (\w+)\$outboundSchema\b")
# The schema's object, up to the `})` indented as the line that opens it
# (Speakeasy indents a declaration with a long name by two more spaces).
OBJECT_RE = re.compile(r"^( *)[^\n]*= z\.object\(\{\n(.*?)^\1\}\)", re.M | re.S)
REMAP_RE = re.compile(r"remap\$\(v, \{\n(.*?)\n\s*\}\);", re.S)
# Keys are quoted when they are not identifiers (`"custom_domains"`).
REMAP_PAIR_RE = re.compile(r'^\s*(?:"([^"]+)"|(\w+)): "([^"]+)",$', re.M)


def object_keys(declaration):
    """The keys of a schema declaration's `z.object` and its `remap$` table
    ({key: renamed}), or None when it has no object."""
    found = OBJECT_RE.search(declaration)
    if not found:
        return None
    key = re.compile(rf'^{found[1]}  (?:"([^"]+)"|(\w+)): ', re.M)
    keys = [quoted or plain for quoted, plain in key.findall(found[2])]
    pairs = REMAP_PAIR_RE.findall("\n".join(REMAP_RE.findall(declaration)))
    return keys, {quoted or plain: renamed for quoted, plain, renamed in pairs}


def camel(name):
    parts = [p for p in re.split(r"[^0-9A-Za-z]+", name) if p]
    if not parts:
        return name
    head = parts[0][:1].lower() + parts[0][1:]
    return head + "".join(p[:1].upper() + p[1:] for p in parts[1:])


def clean_type(text):
    """Collapse a (possibly multi-line) TS type and drop a trailing `| undefined`."""
    t = " ".join(text.split()).rstrip(",").strip()
    t = re.sub(r"^\|\s*", "", t)
    return re.sub(r"\s*\|\s*undefined$", "", t)


def parse_params(block):
    params = []
    for line in block.split("\n"):
        m = PARAM_START_RE.match(line)
        if m:
            params.append({"name": m[1], "optional": m[2] == "?", "type": m[3]})
        else:
            params[-1]["type"] += " " + line
    for p in params:
        p["type"] = clean_type(p["type"])
    return [p for p in params if p["name"] != "options"]


def parse_sdk(ts_dir):
    """Return {operationId: exposure} for every method reachable from the root client."""
    classes, root = {}, None
    for f in sorted((ts_dir / "src/sdk").glob("*.ts")):
        text = f.read_text()
        cm = CLASS_RE.search(text)
        if not cm:
            continue
        if f.name == "sdk.ts":
            root = cm[1]
        imports = dict(FUNC_IMPORT_RE.findall(text))
        classes[cm[1]] = {
            "getters": GETTER_RE.findall(text),
            "methods": [(m[1], m[2], m[3], imports[m[4]]) for m in METHOD_RE.finditer(text)],
        }

    group_of, stack = {root: None}, [root]
    while stack:
        cls = stack.pop()
        for prop, child in classes[cls]["getters"]:
            if child in group_of:
                raise SystemExit(f"{ts_dir}: class {child} reachable via two groups")
            group_of[child] = prop if group_of[cls] is None else f"{group_of[cls]}.{prop}"
            stack.append(child)
    orphans = sorted(c for c, v in classes.items() if v["methods"] and c not in group_of)
    if orphans:
        raise SystemExit(f"{ts_dir}: classes with methods not reachable from {root}: {orphans}")

    exposures = {}
    for cls, group in group_of.items():
        for name, block, ret, func_file in classes[cls]["methods"]:
            func_text = (ts_dir / "src/funcs" / f"{func_file}.ts").read_text()
            (op_id,) = OPERATION_ID_RE.findall(func_text)
            (http_method,) = HTTP_METHOD_RE.findall(func_text)
            (path,) = PATH_RE.findall(func_text)
            if op_id in exposures:
                raise SystemExit(f"{ts_dir}: operation {op_id} exposed twice")
            params = parse_params(block)
            request_schemas = sorted(set(REQUEST_SCHEMA_RE.findall(func_text)))
            if len(request_schemas) > 1:
                raise SystemExit(f"{func_file}: validates several request types: {request_schemas}")
            exposures[op_id] = {
                "group": group,
                "method_name": name,
                "call_shape": [p["name"] for p in params],
                "params": params,
                "request_type": params[-1]["type"] if params else None,
                "response_type": clean_type(ret),
                "sdk_class": cls,
                "sdk_func": func_file,
                "sdk_http_method": http_method,
                "sdk_path": path,
                "request_schema": f"operations.{request_schemas[0]}" if request_schemas else None,
            }
    return root, exposures


def parse_field_names(ts_dir):
    """{wire name: SDK name} for every key of a model's inbound (response)
    schema that the SDK renames or that is not plain letters and digits."""
    names = {}
    for f in sorted((ts_dir / "src/models").rglob("*.ts")):
        for declaration in EXPORT_RE.findall(f.read_text()):
            schema = INBOUND_RE.match(declaration)
            found = object_keys(declaration)
            if not schema or not found:
                continue
            keys, remap = found
            for wire in keys:
                name = remap.get(wire, wire)
                if name == wire and wire.isalnum():
                    continue
                if names.setdefault(wire, name) != name:
                    raise SystemExit(f"{f}: {wire!r} is {names[wire]!r} elsewhere, {name!r} in {schema[1]}")
    return dict(sorted(names.items()))


def parse_request_keys(ts_dir):
    """{request type: {wire name: SDK name}} for every key of an operation
    request type's outbound schema (`operations.<Op>Request`)."""
    out = {}
    for f in sorted((ts_dir / "src/models/operations").glob("*.ts")):
        for declaration in EXPORT_RE.findall(f.read_text()):
            schema = OUTBOUND_RE.match(declaration)
            found = object_keys(declaration)
            if not schema or not found:
                continue
            keys, remap = found
            out[f"operations.{schema[1]}"] = {remap.get(name, name): name for name in keys}
    return out


def parse_spec(path):
    spec = json.loads(path.read_text())

    def resolve(param):
        while "$ref" in param:
            node = spec
            for part in param["$ref"].lstrip("#/").split("/"):
                node = node[part.replace("~1", "/").replace("~0", "~")]
            param = node
        return param

    ops = {}
    for p, item in spec["paths"].items():
        for method in HTTP_METHODS:
            if method in item:
                op = item[method]
                params = [resolve(x)["name"] for x in item.get("parameters", []) + op.get("parameters", [])]
                ops[op["operationId"]] = {"path": p, "method": method.upper(), "op": op, "params": params}
    return ops


def predict(sdk, op):
    if sdk == "platform":
        return op.get("x-speakeasy-group"), op.get("x-speakeasy-name-override") or camel(op["operationId"])
    tags = op.get("tags") or []
    return (camel(tags[0]) if tags else None), camel(op["operationId"])


def gen_lock_versions(ts_dir):
    text = (ts_dir / ".speakeasy/gen.lock").read_text()
    keys = ("speakeasyVersion", "generationVersion", "releaseVersion")
    return {k: (re.search(rf"^\s+{k}: (\S+)", text, re.M) or [None, None])[1] for k in keys}


def speakeasy_overlays(alien_ref, ts_dir):
    """Overlays the Speakeasy workflow applies, as {repo-relative path: text}."""
    workflow = (ts_dir / ".speakeasy/workflow.yaml").read_text()
    rels = [os.path.relpath(os.path.normpath(ts_dir / loc), alien_ref)
            for loc in re.findall(r"location: (\S+)", workflow) if "overlay" in Path(loc).name]
    return {rel: (alien_ref / rel).read_text() for rel in rels}


def explain(op, path, method, actual, overlays):
    """Mechanical notes on why a spec-based prediction missed (not used for prediction)."""
    why, tags = [], op.get("tags") or []
    if "x-speakeasy-group" not in op and tags and actual["group"] == camel(tags[0]):
        why.append("no x-speakeasy-group; SDK group is camelCase(tags[0])")
    target = f"['{path}']['{method.lower()}']"
    why += [f"overlay {rel} targets this operation" for rel, text in overlays.items() if target in text]
    return why


def build_sdk(alien_ref, sdk):
    base = alien_ref / SDKS[sdk]
    ts_dir = base / "typescript"
    spec_ops = parse_spec(base / "openapi.json")
    root, exposures = parse_sdk(ts_dir)
    request_keys = parse_request_keys(ts_dir)
    overlays = speakeasy_overlays(alien_ref, ts_dir)
    sdk_fields = ("call_shape", "group", "method_name", "params", "request_type",
                  "response_type", "sdk_class", "sdk_func")

    operations, naming, spec_only, route_diffs = {}, [], [], []
    for op_id, s in sorted(spec_ops.items()):
        e = exposures.get(op_id)
        entry = {"path": s["path"], "method": s["method"], "in_sdk": e is not None}
        entry.update({k: (e[k] if e else None) for k in sdk_fields})
        # The SDK's name for each spec parameter (its request type's other key
        # is the body).
        if e:
            keys = request_keys[e["request_schema"]] if e["request_schema"] else {}
            entry["param_names"] = {wire: name for wire, name in keys.items() if wire in s["params"]}
            missing = [w for w in s["params"] if w not in entry["param_names"] and w != "workspace"]
            if missing:
                raise SystemExit(f"{sdk}.{op_id}: parameters {missing} not in {e['request_schema']}")
        else:
            entry["param_names"] = None
        operations[op_id] = entry
        if e is None:
            spec_only.append({"operation_id": op_id, "path": s["path"], "method": s["method"],
                              "x-speakeasy-ignore": bool(s["op"].get("x-speakeasy-ignore"))})
            continue
        if (e["sdk_path"], e["sdk_http_method"]) != (s["path"], s["method"]):
            route_diffs.append({"operation_id": op_id, "spec": [s["method"], s["path"]],
                                "sdk": [e["sdk_http_method"], e["sdk_path"]]})
        p_group, p_method = predict(sdk, s["op"])
        differs = [k for k, a, b in (("group", p_group, e["group"]),
                                     ("method_name", p_method, e["method_name"])) if a != b]
        if differs:
            actual = {"group": e["group"], "method_name": e["method_name"]}
            naming.append({
                "operation_id": op_id,
                "differs": differs,
                "predicted": {"group": p_group, "method_name": p_method},
                "actual": actual,
                "explained_by": explain(s["op"], s["path"], s["method"], actual, overlays),
                "spec_hints": {k: s["op"].get(k) for k in
                               ("tags", "x-speakeasy-group", "x-speakeasy-name-override")},
            })

    sdk_only = []
    for op_id in sorted(set(exposures) - set(spec_ops)):
        e = exposures[op_id]
        found = [rel for rel, text in overlays.items() if f'"{op_id}"' in text]
        sdk_only.append({"operation_id": op_id, "path": e["sdk_path"], "method": e["sdk_http_method"],
                         "found_in_overlays": found, **{k: e[k] for k in sdk_fields}})

    package = json.loads((ts_dir / "package.json").read_text())
    meta = {
        "package": package["name"],
        "package_version": package["version"],
        "root_client_class": root,
        "spec": f"{SDKS[sdk]}/openapi.json",
        "sdk_dir": f"{SDKS[sdk]}/typescript",
        "speakeasy": {**gen_lock_versions(ts_dir), "overlays": list(overlays)},
        "counts": {
            "spec_operations": len(spec_ops),
            "sdk_methods": len(exposures),
            "mapped": len(spec_ops) - len(spec_only),
            "spec_only": len(spec_only),
            "sdk_only": len(sdk_only),
            "naming_mismatches": len(naming),
            "route_mismatches": len(route_diffs),
        },
    }
    mismatches = {"prediction_rule": PREDICTION_RULES[sdk], "naming": naming,
                  "spec_only": spec_only, "sdk_only": sdk_only, "route": route_diffs}
    return meta, operations, mismatches, parse_field_names(ts_dir)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--alien-ref", required=True, type=Path, help="alien repo checkout")
    ap.add_argument("--out", required=True, type=Path, help="output JSON path")
    args = ap.parse_args()

    commit = subprocess.run(["git", "-C", str(args.alien_ref), "rev-parse", "HEAD"],
                            capture_output=True, text=True).stdout.strip() or None
    oracle = {"meta": {"alien_commit": commit, "sdks": {}},
              "operations": {}, "spec_vs_sdk_mismatches": {}, "field_names": {}}
    for sdk in SDKS:
        meta, ops, mismatches, field_names = build_sdk(args.alien_ref, sdk)
        oracle["meta"]["sdks"][sdk] = meta
        oracle["operations"][sdk] = ops
        oracle["spec_vs_sdk_mismatches"][sdk] = mismatches
        oracle["field_names"][sdk] = field_names

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(oracle, indent=2, sort_keys=True, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
