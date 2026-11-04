"""Runs every conformance scenario (conformance/scenarios) through the facade."""

from __future__ import annotations

import ast
import asyncio
import functools
import importlib
import inspect
import json
import os
import re
import typing
from pathlib import Path
from typing import Any, Callable

import pydantic
import pytest

import alien_sdk
from alien_sdk import Alien, AlienError, AlienManager, AsyncAlien, AsyncAlienManager, platform
from alien_sdk._errors import _converted

# The generated package, found through the facade's one import of it (alien_sdk/_gen.py).
baml_sdk = importlib.import_module(platform.__name__.rpartition(".")[0])
compat: Any = importlib.import_module(f"{baml_sdk.__name__}.compat")

REPO_ROOT = Path(__file__).resolve().parents[3]


def load_scenarios() -> list[dict]:
    """Scenarios with phase <= CONFORMANCE_PHASE (every phase when unset)."""
    max_phase = float(os.environ["CONFORMANCE_PHASE"]) if os.environ.get("CONFORMANCE_PHASE") else float("inf")
    scenarios = []
    for path in sorted((REPO_ROOT / "conformance" / "scenarios").glob("*.json")):
        scenario = json.loads(path.read_text(encoding="utf-8"))
        if scenario["phase"] <= max_phase:
            scenarios.append(scenario)
    return scenarios


SCENARIOS = load_scenarios()


def _alien(client: dict, cls: type[Alien | AsyncAlien] = Alien) -> Alien | AsyncAlien:
    return cls(client["api_key"], server_url=client["base_url"], workspace=client.get("workspace"), timeout_ms=client.get("timeout_ms"))


def _alien_manager(client: dict, cls: type[AlienManager | AsyncAlienManager] = AlienManager) -> AlienManager | AsyncAlienManager:
    return cls(client["base_url"], client.get("token"), timeout_ms=client.get("timeout_ms"))


def _platform_client(client: dict) -> platform.PlatformClient:
    return platform.PlatformClient(
        base_url=client["base_url"], api_key=client["api_key"], workspace=client.get("workspace"), timeout_ms=client.get("timeout_ms")
    )


def _converting(function: Callable[..., Any], *args: Any, **kwargs: Any) -> Any:
    """Calls a generated function with the facade's error conversion."""
    with _converted():
        return function(*args, **kwargs)


def _bootstrap_request(request: dict) -> Any:
    if request["role"] == "receiver":
        return platform.CommandBootstrapRequestReceiver(**request)
    return platform.CommandBootstrapRequestSender(**request)


CALLS: dict[str, Callable[[dict], Any]] = {
    "platform.createSetupLink": lambda a: _alien(a["client"]).setup_links.create(platform.CreateSetupLinkRequest(**a["request"])),
    "platform.getProjectTemplateUrls": lambda a: _alien(a["client"]).projects.get_template_urls(a["idOrName"]),
    "platform.bootstrapCommands": lambda a: _alien(a["client"]).commands.bootstrap(_bootstrap_request(a["request"])),
    "platform.getManager": lambda a: _alien(a["client"]).managers.get(a["id"]),
    "manager.getCommandStatus": lambda a: _alien_manager(a["client"]).commands.get_command_status(a["command_id"]),
    "setup.cloudformationLaunchUrl": lambda a: alien_sdk.cloudformation_launch_url(alien_sdk.CloudFormationLaunch(**a["input"])),
    "setup.launchUrlForProject": lambda a: _alien(a["client"]).setup.launch_url_for_project(a["project"], a["region"], a["stackName"], a["parameters"]),
    "compat.acceptKeySetup": lambda a: _converting(compat.accept_key_setup_sync, _platform_client(a["client"]), compat.KeySetupAction(**a["request"])),
    "compat.decodeProjectPage": lambda a: _converting(compat.decode_project_page_sync, a["json"]),
    "compat.decodeMaterialization": lambda a: _converting(compat.decode_materialization_sync, a["json"]),
    "compat.storedSecretPresence": lambda a: _converting(compat.stored_secret_presence_sync, a["json"]),
    "compat.reencodeConfig": lambda a: _converting(compat.reencode_config_sync, a["json"]),
    "compat.roundtripStackByPlatform": lambda a: _converting(compat.roundtrip_stack_by_platform_sync, a["json"]),
    "compat.roundtripEvent": lambda a: _converting(compat.roundtrip_event_sync, a["json"]),
    "compat.roleScope": lambda a: _converting(compat.role_scope_sync, a["json"]),
    "compat.decodeRemoteAccess": lambda a: _converting(compat.decode_remote_access_sync, a["json"]),
}


# The scenarios that call the Platform API through the client, again with AsyncAlien.
ASYNC_CALLS: dict[str, Callable[[dict], Any]] = {
    "platform.createSetupLink": lambda a: _alien(a["client"], AsyncAlien).setup_links.create(platform.CreateSetupLinkRequest(**a["request"])),
    "platform.getProjectTemplateUrls": lambda a: _alien(a["client"], AsyncAlien).projects.get_template_urls(a["idOrName"]),
    "platform.bootstrapCommands": lambda a: _alien(a["client"], AsyncAlien).commands.bootstrap(_bootstrap_request(a["request"])),
    "platform.getManager": lambda a: _alien(a["client"], AsyncAlien).managers.get(a["id"]),
    "manager.getCommandStatus": lambda a: _alien_manager(a["client"], AsyncAlienManager).commands.get_command_status(a["command_id"]),
    "setup.launchUrlForProject": lambda a: _alien(a["client"], AsyncAlien).setup.launch_url_for_project(a["project"], a["region"], a["stackName"], a["parameters"]),
}
ASYNC_SCENARIOS = [s for s in SCENARIOS if s["call"].get("fn") in ASYNC_CALLS]


# BAML types a scenario argument may have that need no conversion.
_PLAIN_TYPES = {"string", "int", "float", "bool", "string[]", "int[]"}


def _host_type(service: str, baml_type: str) -> Any:
    """The generated Python type of a BAML argument type: a class or alias
    name, `T[]`, or a union `(A | B)`."""
    if baml_type.endswith("[]"):
        return list[_host_type(service, baml_type[:-2])]
    if baml_type.startswith("(") and baml_type.endswith(")"):
        members = tuple(_host_type(service, m.strip()) for m in baml_type[1:-1].split("|"))
        return typing.Union[members]
    return getattr(getattr(baml_sdk, service), baml_type)


@functools.cache
def _adapter(service: str, baml_type: str) -> pydantic.TypeAdapter:
    return pydantic.TypeAdapter(_host_type(service, baml_type))


_DEFINITION = re.compile(
    r'^(\w+)\s*= _define_function\("user\.([\w.]+)", "async",\s*(\[[^\]]*\])(?:, (\[[^\]]*\]))?(?:, param_aliases=(\{[^}]*\}))?',
    re.M,
)


@functools.cache
def _bindings() -> dict[str, tuple[list[str], str, dict[str, str]]]:
    """Every generated operation by its BAML path (``platform.deploymentGroups.createToken``):
    its group module and function as the generator named them (``["deployment_groups"]``,
    ``create_token``), and its keyword names by BAML name (``{"deploymentGroup":
    "deployment_group"}``). Read from the generated modules, which record each
    function's BAML path."""
    root = Path(baml_sdk.__file__).parent
    out = {}
    for service in ("platform", "manager"):
        for init in sorted((root / service).rglob("__init__.py")):
            groups = list(init.parent.relative_to(root / service).parts)
            for name, path, _required, optional, aliases in _DEFINITION.findall(init.read_text(encoding="utf-8")):
                wire = ast.literal_eval(aliases) if aliases else {}
                keywords = {wire.get(python, python): python for python in ast.literal_eval(optional or "[]")}
                out[path] = (groups, name, keywords)
    return out


def _operation(call: dict) -> Any:
    """A generated operation (``call.op``, conformance/README.md), called
    through the facade: ``platform.deploymentGroups.createToken`` is
    ``Alien(...).deployment_groups.create_token``, with its required arguments
    in order and the optional ones that are set as keywords, under the
    generated names. Class arguments are built as the generated pydantic types."""
    service = call["op"].split(".")[0]
    groups, name, keywords_by_wire = _bindings()[call["op"]]
    args = call["args"]
    target: Any = _alien_manager(args["client"]) if service == "manager" else _alien(args["client"])
    for group in groups:
        target = getattr(target, group)
    function = getattr(target, name)
    positional, keywords = [], {}
    for param in call["params"]:
        if param["name"] not in args:
            assert param["optional"], f"scenario has no value for required argument {param['name']}"
            continue
        value = args[param["name"]]
        if param["type"] not in _PLAIN_TYPES:
            value = _adapter(service, param["type"]).validate_python(value)
        if param["optional"]:
            keywords[keywords_by_wire[param["name"]]] = value
        else:
            positional.append(value)
    return function(*positional, **keywords)


def test_every_operation_is_bound() -> None:
    """Every operation the scenarios call resolves through the facade: 279
    Platform and 53 Manager operations, each a method with the generated
    function's docs (test_facade.py walks what each calls)."""
    bindings = _bindings()
    assert sum(op.startswith("platform.") for op in bindings) == 279
    assert sum(op.startswith("manager.") for op in bindings) == 53
    clients = {"platform": Alien("key", server_url="http://127.0.0.1:1"), "manager": AlienManager("http://127.0.0.1:1")}
    for op, (groups, name, _) in bindings.items():
        service = op.split(".")[0]
        target: Any = clients[service]
        generated: Any = getattr(baml_sdk, service)
        for group in groups:
            target, generated = getattr(target, group), getattr(generated, group)
        assert inspect.getdoc(getattr(target, name)) == inspect.getdoc(getattr(generated, name)), op


def _to_json(value: Any) -> Any:
    if isinstance(value, pydantic.BaseModel):
        return value.model_dump(mode="json", by_alias=True)  # the BAML names
    if isinstance(value, list):
        return [_to_json(v) for v in value]
    if isinstance(value, dict):
        return {k: _to_json(v) for k, v in value.items()}
    return value


def _error(error: AlienError) -> dict:
    """An AlienError as an outcome for the stub server."""
    return {
        "error": {
            "code": error.code,
            "retryable": error.retryable,
            "http_status": error.http_status,
            "message": error.message,
            "context": error.context,
        }
    }


def test_found_scenarios() -> None:
    assert SCENARIOS


@pytest.mark.parametrize("scenario", SCENARIOS, ids=[s["id"] for s in SCENARIOS])
def test_scenario(stub, scenario: dict) -> None:
    call = stub.post("/__load", {"scenario": scenario})["call"]
    if "op" in call:
        run = functools.partial(_operation, call)
    else:
        named = CALLS.get(call["fn"])
        assert named is not None, f"no Python binding for scenario fn {call['fn']}"
        run = functools.partial(named, call["args"])
    raw = None
    try:
        raw = run()
        outcome: dict = {"ok": _to_json(raw)}
    except AlienError as error:
        outcome = _error(error)
    verdict = stub.post("/__result", outcome)
    out = os.environ.get("CONFORMANCE_OUT")
    if out:
        directory = Path(out) / "python"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / f"{scenario['id']}.json").write_text(json.dumps(verdict["normalized"]) + "\n", encoding="utf-8")
        if scenario.get("capture_raw"):
            (directory / f"{scenario['id']}.txt").write_bytes(str(raw).encode("utf-8"))
    assert not verdict["problems"], "\n".join(verdict["problems"])
    assert verdict["ok"] is True


@pytest.mark.parametrize("scenario", ASYNC_SCENARIOS, ids=[s["id"] for s in ASYNC_SCENARIOS])
def test_scenario_async(stub, scenario: dict) -> None:
    call = stub.post("/__load", {"scenario": scenario})["call"]
    try:
        outcome: dict = {"ok": _to_json(asyncio.run(ASYNC_CALLS[call["fn"]](call["args"])))}
    except AlienError as error:
        outcome = _error(error)
    verdict = stub.post("/__result", outcome)
    assert not verdict["problems"], "\n".join(verdict["problems"])
    assert verdict["ok"] is True
