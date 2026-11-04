"""genops writes ``alien_sdk/_operations.py`` (gitignored): the Python facade's
methods, one per generated Platform and Manager operation, with the generated
signatures.

    uv run --frozen python genops.py      (in sdk/python; scripts/generate.sh runs it)

For each generated group module (``platform/deployments``) it writes two
classes, ``PlatformDeployments`` (blocking: each method calls the generated
``<name>_sync``) and ``AsyncPlatformDeployments`` (``async``: each method
awaits the generated ``<name>``), with the same methods. The service roots
(``Platform``, ``AsyncPlatform``, ``Manager``, ``AsyncManager``) hold the
operations without a group and one attribute per group; ``alien_sdk.Alien``
and the other clients derive from them. A method takes the generated
function's arguments without the client, under the same names, kinds,
annotations and docs, so editors and type checkers see each signature and a
wrong keyword raises ``TypeError``. An optional argument defaults to
``UNSET``, which the generated function treats as not passed, so leaving it
out does what leaving it out of the generated call does.

It reads the generated stubs (``__init__.pyi``) as text and imports nothing,
so it holds no per-operation code and takes every name from them. The
generated package is the one ``alien_sdk/_gen.py`` imports (the only module
of the facade that names it), and the generated code reaches the generated
modules through ``_gen`` as well.
"""

from __future__ import annotations

import ast
import builtins
import keyword
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn

HERE = Path(__file__).resolve().parent
FACADE = "alien_sdk"
GEN = HERE / FACADE / "_gen.py"
OUTPUT = HERE / FACADE / "_operations.py"
# The generated services whose operations the facade binds, and their client classes.
SERVICES = {"platform": "PlatformClient", "manager": "ManagerClient"}
# What the generated SDK appends to an operation's (async) name for its blocking twin.
SYNC_SUFFIX = "_sync"
# The per-call options every generated function takes; forwarded as they are.
OPTIONS = "_baml"


def die(message: str) -> NoReturn:
    sys.exit(f"genops: {message}")


def generated_package() -> tuple[str, Path, set[str]]:
    """The generated package's dotted name and directory, from ``_gen.py``'s
    ``from <package> import ...``, and the names ``_gen`` exports from it."""
    tree = ast.parse(GEN.read_text(encoding="utf-8"))
    imports = [n for n in tree.body if isinstance(n, ast.ImportFrom) and n.module != "__future__"]
    if len(imports) != 1:
        die(f"{GEN.relative_to(HERE)} must import the generated package in exactly one `from ... import ...`")
    node = imports[0]
    if node.level:
        package = ".".join([FACADE, *([node.module] if node.module else [])])
        if node.level > 1:
            die("_gen.py imports the generated package more than one level up")
    else:
        package = node.module or ""
    exported = {a.asname or a.name for a in node.names}
    renamed = [a.name for a in node.names if a.asname and a.asname != a.name]
    if renamed:
        die(f"_gen.py renames {renamed}; import the generated modules under their own names")
    directory = HERE.joinpath(*package.split("."))
    if not (directory / "__init__.pyi").is_file() and not (directory / "__init__.py").is_file():
        die(f"no generated package at {directory} (run baml generate first)")
    return package, directory, exported


def pascal(snake: str) -> str:
    return "".join(part[:1].upper() + part[1:] for part in snake.rstrip("_").split("_"))


@dataclass
class Function:
    name: str
    node: ast.AsyncFunctionDef
    doc: str | None


@dataclass
class Module:
    service: str
    group: str | None  # None: the service root
    dotted: str  # the generated module, relative to the package: "platform.deployments"
    functions: list[Function]
    groups: list[str]


class Qualifier(ast.NodeTransformer):
    """Rewrites a stub's annotation to names the generated file has: a
    generated name becomes ``_gen.<module>.<rest>`` (``_gen.platform.Foo``,
    ``_gen.baml.UNSET``), ``typing.X`` becomes ``_typing.X``. Every name then
    starts with ``_``, so no method or group of the class it is used in
    (``Manager.manager``, ``PlatformDeployments.list``) shadows it; a builtin
    that would be is an error."""

    def __init__(self, env: dict[str, str], package: str, exported: set[str], used: set[str], members: set[str]) -> None:
        self.env, self.package, self.exported, self.used, self.members = env, package, exported, used, members

    def visit_Constant(self, node: ast.Constant) -> ast.AST:
        if isinstance(node.value, str):
            die(f"a string annotation ({node.value!r}) in a generated stub; genops does not read those")
        return node

    def visit_Name(self, node: ast.Name) -> ast.AST:
        path = self.env.get(node.id)
        if path is None:
            if hasattr(builtins, node.id):
                if node.id in self.members:
                    die(f"a method or group named {node.id!r} shadows the builtin in an annotation")
                return node
            die(f"generated stubs use {node.id!r}, which genops cannot resolve")
        if path in {"typing", "typing_extensions"}:
            self.used.add(path)
            return ast.Name(id=f"_{path}", ctx=node.ctx)
        if not path.startswith(self.package + "."):
            die(f"generated stubs use {path}, outside the generated package")
        first, *rest = path[len(self.package) + 1 :].split(".")
        if first not in self.exported:
            die(f"generated stubs use {path}: add `{first}` to the imports of _gen.py")
        expr: ast.expr = ast.Attribute(value=ast.Name(id="_gen", ctx=ast.Load()), attr=first, ctx=ast.Load())
        for part in rest:
            expr = ast.Attribute(value=expr, attr=part, ctx=ast.Load())
        return expr


def environment(tree: ast.Module, module: str, is_package: bool) -> dict[str, str]:
    """Each top-level name of a stub (imports, also under ``if TYPE_CHECKING``,
    and definitions) by the dotted path it stands for."""
    env: dict[str, str] = {}
    base = module.split(".") if is_package else module.split(".")[:-1]

    def walk(body: list[ast.stmt]) -> None:
        for node in body:
            if isinstance(node, ast.If):
                walk(node.body)
                walk(node.orelse)
            elif isinstance(node, ast.Import):
                for a in node.names:
                    env[a.asname or a.name.split(".")[0]] = a.name if a.asname else a.name.split(".")[0]
            elif isinstance(node, ast.ImportFrom):
                if node.module == "__future__":
                    continue
                if node.level:
                    parent = base[: len(base) - (node.level - 1)]
                    source = ".".join([*parent, *([node.module] if node.module else [])])
                else:
                    source = node.module or ""
                for a in node.names:
                    env[a.asname or a.name] = f"{source}.{a.name}"
            elif isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
                env[node.name] = f"{module}.{node.name}"
            elif isinstance(node, ast.Assign):
                for target in node.targets:
                    if isinstance(target, ast.Name):
                        env[target.id] = f"{module}.{target.id}"
            elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
                env[node.target.id] = f"{module}.{node.target.id}"
            elif sys.version_info >= (3, 12) and isinstance(node, ast.TypeAlias):
                env[node.name.id] = f"{module}.{node.name.id}"

    walk(tree.body)
    return env


def read_module(package: str, directory: Path, service: str, group: str | None) -> tuple[Module, dict[str, str]]:
    path = directory / service / (group or "") / "__init__.pyi"
    tree = ast.parse(path.read_text(encoding="utf-8"))
    dotted = f"{service}.{group}" if group else service
    env = environment(tree, f"{package}.{dotted}", is_package=True)
    defs = {n.name: n for n in tree.body if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))}
    functions = []
    for name, node in defs.items():
        if not isinstance(node, ast.AsyncFunctionDef) or name.startswith("_"):
            continue
        twin = defs.get(name + SYNC_SUFFIX)
        if not isinstance(twin, ast.FunctionDef):
            die(f"{dotted}.{name} has no blocking twin {name + SYNC_SUFFIX}")
        if ast.dump(twin.args) != ast.dump(node.args) or ast.dump(twin.returns) != ast.dump(node.returns):  # type: ignore[arg-type]
            die(f"{dotted}.{name} and {name + SYNC_SUFFIX} have different signatures")
        check(dotted, name, node, SERVICES[service])
        functions.append(Function(name, node, ast.get_docstring(node, clean=True)))
    groups = sorted(
        p.name for p in (directory / service).iterdir() if group is None and (p / "__init__.pyi").is_file()
    )
    return Module(service, group, dotted, functions, groups), env


def check(dotted: str, name: str, f: ast.AsyncFunctionDef, client: str) -> None:
    """What the generated methods rely on: the client first, required
    arguments positional, optional ones keyword-only defaulting to None (and
    accepting UNSET), the per-call options last, nothing generic."""
    a = f.args
    where = f"{dotted}.{name}"
    if a.posonlyargs or a.vararg or a.kwarg or a.defaults or getattr(f, "type_params", None):
        die(f"{where}: an unexpected signature ({ast.unparse(a)})")
    if not a.args or a.args[0].arg != "client":
        die(f"{where}: the first argument is not the client")
    annotation = a.args[0].annotation
    if annotation is None or ast.unparse(annotation).split(".")[-1] != client:
        die(f"{where}: the client is not a {client}")
    names = [arg.arg for arg in [*a.args[1:], *a.kwonlyargs]]
    if not a.kwonlyargs or a.kwonlyargs[-1].arg != OPTIONS:
        die(f"{where}: {OPTIONS} is not its last argument")
    for n in names[:-1]:
        if n.startswith("_") or n == "self" or keyword.iskeyword(n):
            die(f"{where}: argument {n!r} cannot be a method argument")
    for arg, default in zip(a.kwonlyargs[:-1], a.kw_defaults[:-1]):
        if default is None or ast.unparse(default) != "None" or "UNSET" not in ast.unparse(arg.annotation or ast.Constant(None)):
            die(f"{where}: optional argument {arg.arg} does not default to None and accept UNSET")


def method(m: Module, f: Function, asynchronous: bool, qualify: Qualifier, indent: str = "    ") -> list[str]:
    a = f.node.args
    positional = a.args[1:]
    keywords = a.kwonlyargs
    params = ["self"]
    for arg in positional:
        params.append(f"{arg.arg}: {annotation(arg, qualify)}")
    params.append("*")
    for arg in keywords:
        # UNSET, not the stub's None: the generated function drops an UNSET
        # argument, so the default does what leaving the argument out does.
        default = "None" if arg.arg == OPTIONS else unset(qualify)
        params.append(f"{arg.arg}: {annotation(arg, qualify)} = {default}")
    returns = ast.unparse(qualify.visit(copy(f.node.returns))) if f.node.returns else "None"
    target = f"_gen.{m.dotted}.{f.name}" + ("" if asynchronous else SYNC_SUFFIX)
    call_args = ["self._client", *(arg.arg for arg in positional), *(f"{arg.arg}={arg.arg}" for arg in keywords)]
    call = f"{target}({', '.join(call_args)})"
    lines = [f"{indent}{'async ' if asynchronous else ''}def {f.name}(", *(f"{indent}    {p}," for p in params), f"{indent}) -> {returns}:"]
    body = indent * 2
    if f.doc:
        lines.append(docstring(f.doc, body))
    lines += [f"{body}with _converted():", f"{body}    return {'await ' if asynchronous else ''}{call}"]
    return lines


def copy(node: ast.expr | None) -> ast.expr:
    assert node is not None
    return ast.parse(ast.unparse(node), mode="eval").body


def annotation(arg: ast.arg, qualify: Qualifier) -> str:
    if arg.annotation is None:
        die(f"argument {arg.arg} has no annotation")
    return ast.unparse(qualify.visit(copy(arg.annotation)))


def docstring(text: str, indent: str) -> str:
    text = text.replace("\\", "\\\\").replace('"""', '\\"\\"\\"')
    lines = text.splitlines()
    out = [f'{indent}"""{lines[0]}' if lines else f'{indent}"""']
    out += [f"{indent}{line}" if line else "" for line in lines[1:]]
    if len(lines) > 1:
        out.append(f'{indent}"""')
    else:
        out[-1] += '"""'
    return "\n".join(out)


def class_name(m: Module, asynchronous: bool) -> str:
    return ("Async" if asynchronous else "") + pascal(m.service) + (pascal(m.group) if m.group else "")


def write_class(m: Module, groups: list[Module], asynchronous: bool, qualify: Qualifier) -> list[str]:
    name = class_name(m, asynchronous)
    client = f"_gen.{m.service}.{SERVICES[m.service]}"
    mode = "``async`` methods, awaiting" if asynchronous else "blocking methods, calling"
    called = "``<name>``" if asynchronous else f"``<name>{SYNC_SUFFIX}``"
    if m.group:
        doc = f"The ``{m.group}`` operations of the {pascal(m.service)} API, as {mode} the generated {called} of ``{m.dotted}``."
    else:
        doc = (
            f"The {pascal(m.service)} API's operations without a group and one attribute per group, as {mode} "
            f"the generated {called} of ``{m.dotted}``."
        )
    lines = [f"class {name}:", f'    """{doc}"""', ""]
    method_names = {f.name for f in m.functions}
    clash = method_names & {g.group for g in groups}
    if clash:
        die(f"{m.dotted}: {sorted(clash)} are both an operation and a group")
    for g in groups:
        lines.append(f"    {g.group}: {class_name(g, asynchronous)}")
    if groups:
        lines.append("")
    lines += [f"    def __init__(self, client: {client}) -> None:", "        self._client = client"]
    for g in groups:
        lines.append(f"        self.{g.group} = {class_name(g, asynchronous)}(client)")
    for f in m.functions:
        lines.append("")
        lines += method(m, f, asynchronous, qualify)
    return lines


def members(m: Module, groups: list[Module]) -> set[str]:
    """The names a generated class defines: its methods and groups."""
    return {f.name for f in m.functions} | {g.group for g in groups if g.group}


def unset(q: Qualifier) -> str:
    """The generated ``UNSET`` sentinel, as the generated file reaches it (``baml.UNSET``)."""
    if "UNSET" not in q.env:
        die("a generated stub with optional arguments imports no UNSET")
    return ast.unparse(q.visit(ast.Name(id="UNSET", ctx=ast.Load())))


def main() -> None:
    package, directory, exported = generated_package()
    used: set[str] = set()
    classes: list[str] = []
    names: list[str] = []
    counts: dict[str, int] = {}
    for service in SERVICES:
        root, root_env = read_module(package, directory, service, None)
        groups = [read_module(package, directory, service, group) for group in root.groups]
        counts[service] = len(root.functions) + sum(len(g.functions) for g, _ in groups)
        for asynchronous in (False, True):
            for g, env in groups:
                q = Qualifier(env, package, exported, used, members(g, []))
                classes += [*write_class(g, [], asynchronous, q), "", ""]
                names.append(class_name(g, asynchronous))
            group_modules = [g for g, _ in groups]
            q = Qualifier(root_env, package, exported, used, members(root, group_modules))
            classes += [*write_class(root, group_modules, asynchronous, q), "", ""]
            names.append(class_name(root, asynchronous))
    header = [
        "# This file was generated by sdk/python/genops.py (scripts/generate.sh runs it): do not edit it.",
        '"""The facade\'s methods: one per generated Platform and Manager operation',
        f"({counts['platform']} and {counts['manager']}), with the generated signatures (sdk/python/genops.py).",
        '"""',
        "",
        "from __future__ import annotations",
        "",
        *(f"import {m} as _{m}" for m in sorted(used)),
        "",
        "from . import _gen",
        "from ._errors import _converted",
        "",
        "__all__ = [",
        *(f'    "{n}",' for n in sorted(names)),
        "]",
        "",
        "",
    ]
    OUTPUT.write_text("\n".join(header) + "\n".join(classes).rstrip() + "\n", encoding="utf-8")
    print(
        f"genops: wrote {OUTPUT.relative_to(HERE)} ({counts['platform']} Platform and {counts['manager']} Manager operations)",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
