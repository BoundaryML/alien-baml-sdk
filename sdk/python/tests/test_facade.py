"""The facade itself: client options, what is bound, the error type, shutdown.

Each call goes to conformance/stub-server, scripted by the test.
"""

from __future__ import annotations

import asyncio
import copy
import importlib
import inspect
import json
import keyword
import os
import pickle
import pkgutil
import subprocess
import sys
import textwrap
import traceback
import types
from pathlib import Path

import pydantic
import pytest
from baml_bridge import BamlError

import alien_sdk
from alien_sdk import Alien, AlienError, AlienManager, AsyncAlien, AsyncAlienManager, manager, platform
from alien_sdk._bind import _snake


def script(stub, *exchanges: dict) -> None:
    """Loads the requests the next calls must make, and the stub's replies."""
    stub.post("/__load", {"scenario": {"id": "facade", "phase": 0, "call": None, "exchanges": list(exchanges)}})


def requests(stub) -> list[dict]:
    """The requests made since `script`, after checking they were exactly the scripted ones."""
    log = stub.get("/__log")
    assert log["failures"] == []
    assert log["consumed"] == log["total"]
    return log["requests"]


USER = {"kind": "user", "id": "usr_1", "email": "dev@example.com", "workspaceId": "ws_1", "role": "workspace.admin"}


def test_api_key_defaults_to_environment(stub, secrets, monkeypatch) -> None:
    monkeypatch.setenv("ALIEN_API_KEY", secrets["apiKey"])
    script(stub, {
        "expect": {"method": "GET", "path": "/v1/whoami", "headers": {"authorization": f"Bearer {secrets['apiKey']}"}},
        "respond": {"json": USER},
    })
    me = Alien(server_url=stub.base).auth.whoami()  # an operation without arguments
    assert isinstance(me, platform.UserSubject) and me.email == "dev@example.com"
    requests(stub)


def test_api_key_argument_wins_over_environment(stub, secrets, monkeypatch) -> None:
    monkeypatch.setenv("ALIEN_API_KEY", "not_this_one")
    script(stub, {
        "expect": {"method": "GET", "path": "/v1/whoami", "headers": {"authorization": f"Bearer {secrets['apiKey']}"}},
        "respond": {"json": USER},
    })
    Alien(secrets["apiKey"], server_url=stub.base).auth.whoami()
    requests(stub)


@pytest.mark.parametrize("client", [Alien, AsyncAlien])
@pytest.mark.parametrize("api_key", [None, ""])
def test_missing_api_key_fails_at_construction(monkeypatch, client, api_key) -> None:
    monkeypatch.delenv("ALIEN_API_KEY", raising=False)
    with pytest.raises(AlienError) as info:
        client(api_key)
    assert (info.value.code, info.value.retryable) == ("INVALID_REQUEST", False)
    assert "ALIEN_API_KEY" in info.value.message


@pytest.mark.parametrize("client", [Alien, AsyncAlien])
def test_timeout_ms_reaches_the_client(secrets, client) -> None:
    assert client(secrets["apiKey"])._client.timeout_ms is None
    assert client(secrets["apiKey"], timeout_ms=300)._client.timeout_ms == 300
    with pytest.raises(pydantic.ValidationError):
        client(secrets["apiKey"], timeout_ms=1.5)


@pytest.mark.parametrize("client", [AlienManager, AsyncAlienManager])
def test_manager_timeout_ms_reaches_the_client(client) -> None:
    assert client("http://127.0.0.1:9")._client.timeout_ms is None
    assert client("http://127.0.0.1:9", timeout_ms=300)._client.timeout_ms == 300


def test_default_server_url(secrets) -> None:
    assert alien_sdk.DEFAULT_SERVER_URL == "https://api.alien.dev"
    assert repr(Alien(secrets["apiKey"])) == "Alien(server_url='https://api.alien.dev', workspace=None)"
    # None means the default, as an unset serverURL does in the other SDKs.
    assert repr(Alien(secrets["apiKey"], server_url=None)) == "Alien(server_url='https://api.alien.dev', workspace=None)"
    assert repr(AsyncAlien(secrets["apiKey"])) == "AsyncAlien(server_url='https://api.alien.dev', workspace=None)"


def test_keyword_arguments_keep_wire_names(stub, secrets) -> None:
    alien = Alien(secrets["apiKey"], server_url=stub.base, workspace="acme")
    script(stub, {
        "expect": {"method": "GET", "path": "/v1/deployments?workspace=acme&project=web&deploymentGroup=prod&limit=2"},
        "respond": {"json": {"items": [], "nextCursor": "c2"}},
    })
    page = alien.deployments.list(project="web", deploymentGroup="prod", limit=2)
    assert page.nextCursor == "c2"
    requests(stub)

    # A snake_case keyword is not one of the operation's names (docs/api-shape.md).
    script(stub)
    with pytest.raises(BamlError, match="deployment_group"):
        alien.deployments.list(deployment_group="prod")
    assert requests(stub) == []


def test_operation_without_response_body_returns_none(stub, secrets) -> None:
    script(stub, {
        "expect": {"method": "DELETE", "path": "/v1/api-keys/key_1?workspace=acme"},
        "respond": {"status": 204},
    })
    assert Alien(secrets["apiKey"], server_url=stub.base, workspace="acme").api_keys.revoke("key_1") is None
    requests(stub)


def test_manager_sends_its_token(stub, secrets) -> None:
    script(stub, {
        "expect": {
            "method": "POST",
            "path": "/v1/commands",
            "headers": {"authorization": f"Bearer {secrets['managerToken']}"},
            "json": {"command": "generate-report", "deploymentId": "dep_1", "params": {"mode": "inline", "inlineBase64": "e30="}},
        },
        "respond": {"json": {"commandId": "cmd_1", "inlineAllowedUpTo": 1024, "next": "poll", "state": "PENDING"}},
    })
    commands = AlienManager(stub.base, secrets["managerToken"]).commands
    created = commands.create_command(manager.CreateCommandRequest(
        command="generate-report", deploymentId="dep_1", params=manager.BodySpecInline(mode="inline", inlineBase64="e30=")))
    assert created.commandId == "cmd_1"
    requests(stub)


@pytest.mark.parametrize("token", [None, ""])
def test_manager_without_token_sends_no_authorization(stub, token) -> None:
    script(stub, {
        "expect": {"method": "POST", "path": "/v1/commands", "headers_absent": ["authorization"]},
        "respond": {"json": {"commandId": "cmd_1", "inlineAllowedUpTo": 1024, "next": "poll", "state": "PENDING"}},
    })
    AlienManager(stub.base, token).commands.create_command(manager.CreateCommandRequest(
        command="c", deploymentId="d", params=manager.BodySpecInline(mode="inline", inlineBase64="")))
    requests(stub)


def _not_found(stub, secrets) -> AlienError:
    script(stub, {
        "expect": {"method": "GET", "path": "/v1/managers/mgr_x?workspace=acme"},
        "respond": {"status": 404, "json": {
            "code": "NOT_FOUND", "message": "Manager not found.", "retryable": False, "httpStatusCode": 404,
            "hint": "Check the ID.", "requestId": "req_1", "context": {"id": "mgr_x"},
        }},
    })
    with pytest.raises(AlienError) as info:
        Alien(secrets["apiKey"], server_url=stub.base, workspace="acme").managers.get("mgr_x")
    requests(stub)
    return info.value


def test_alien_error(stub, secrets) -> None:
    error = _not_found(stub, secrets)
    assert (error.code, error.retryable, error.http_status) == ("NOT_FOUND", False, 404)
    assert str(error) == "NOT_FOUND: Manager not found."
    assert error.context is not None
    assert (error.context["requestId"], error.context["hint"]) == ("req_1", "Check the ID.")
    assert json.loads(error.context["details"]) == {"id": "mgr_x"}
    assert isinstance(error.__cause__, BamlError)
    assert f"{type(error).__module__}.{type(error).__qualname__}" == "alien_sdk.AlienError"
    for clone in (pickle.loads(pickle.dumps(error)), copy.copy(error), copy.deepcopy(error)):
        assert type(clone) is AlienError
        fields = ("code", "message", "retryable", "http_status", "context")
        assert [getattr(clone, f) for f in fields] == [getattr(error, f) for f in fields]


def test_a_union_tagged_by_two_fields_has_one_catch_all(stub, secrets) -> None:
    # A deployment with an S3 binding and a `storage` binding whose `service`,
    # `minio`, this SDK does not know (docs/api-shape.md, "Unions").
    scenario = Path(__file__).parents[3] / "conformance/scenarios/ops.platform.deployments.get.external_binding_new_service.json"
    script(stub, {
        "expect": {"method": "GET", "path": "/v1/deployments/dep_1"},
        "respond": json.loads(scenario.read_text())["exchanges"][0]["respond"],
    })
    deployment = Alien(secrets["apiKey"], server_url=stub.base).deployments.get("dep_1")
    requests(stub)
    got = []
    for binding in deployment.stackSettings.externalBindings.values():
        match binding:
            case platform.ExternalBindingOther(raw=raw):
                got.append(raw["service"])
            case platform.ExternalBindingStorageS3(bucketName=bucket):
                got.append(bucket)
    assert got == ["files", "minio"]


def test_credentials_stay_out_of_reprs(stub, secrets) -> None:
    alien = Alien(secrets["apiKey"], server_url=stub.base, workspace="acme")
    manager_client = AlienManager(stub.base, secrets["managerToken"])
    async_alien = AsyncAlien(secrets["apiKey"], server_url=stub.base, workspace="acme")
    async_manager = AsyncAlienManager(stub.base, secrets["managerToken"])
    for value in (alien, alien.managers, manager_client, manager_client.commands,
                  async_alien, async_alien.managers, async_manager, async_manager.commands):
        for text in (repr(value), str(value), repr(vars(value))):
            assert secrets["apiKey"] not in text and secrets["managerToken"] not in text

    # Tracebacks that capture locals, as error reporters do: the only local
    # holding the key is the encoded call that baml_bridge keeps in its own
    # frame, which the facade cannot reach.
    error = _not_found(stub, secrets)
    holding = set()
    stack: traceback.TracebackException | None = traceback.TracebackException.from_exception(error, capture_locals=True)
    while stack is not None:
        for frame in stack.stack:
            if Path(frame.filename).parent == Path(__file__).parent:
                continue  # this test's own frames
            holding |= {f"{Path(frame.filename).parent.name}.{frame.name}:{name}"
                        for name, value in (frame.locals or {}).items() if secrets["apiKey"] in value}
        stack = stack.__cause__ or (None if stack.__suppress_context__ else stack.__context__)
    assert holding <= {"baml_bridge._sync:args_proto"}


def _operations(module: types.ModuleType) -> set[str]:
    return {
        name for name, value in vars(module).items()
        if isinstance(value, types.FunctionType) and value.__module__ == module.__name__
        and not name.startswith("_") and not name.endswith("_async")
    }


@pytest.mark.parametrize("module", [platform, manager], ids=lambda m: m.__name__)
def test_every_generated_operation_is_bound(secrets, module: types.ModuleType) -> None:
    client = Alien(secrets["apiKey"], server_url="http://127.0.0.1:1") if module is platform else AlienManager("http://127.0.0.1:1")
    groups = {info.name for info in pkgutil.iter_modules(module.__path__) if info.ispkg}
    assert groups
    own = {"setup"} if module is platform else set()
    assert not own & {_snake(group) for group in groups}, "a generated group is shadowed by a facade attribute"
    assert set(dir(client)) == {_snake(group) for group in groups} | {_snake(op) for op in _operations(module)} | own
    for group in groups:
        generated = importlib.import_module(f"{module.__name__}.{group}")
        bound = getattr(client, _snake(group))
        assert set(dir(bound)) == {_snake(op) for op in _operations(generated)}, group
        for op in _operations(generated):
            assert getattr(bound, _snake(op)).__wrapped__ is getattr(generated, op)


@pytest.mark.parametrize("module", [platform, manager], ids=lambda m: m.__name__)
def test_async_clients_bind_the_async_functions(secrets, module: types.ModuleType) -> None:
    client = AsyncAlien(secrets["apiKey"], server_url="http://127.0.0.1:1") if module is platform else AsyncAlienManager("http://127.0.0.1:1")
    groups = {info.name for info in pkgutil.iter_modules(module.__path__) if info.ispkg}
    own = {"setup"} if module is platform else set()
    assert set(dir(client)) == {_snake(group) for group in groups} | {_snake(op) for op in _operations(module)} | own
    for group in groups:
        generated = importlib.import_module(f"{module.__name__}.{group}")
        bound = getattr(client, _snake(group))
        assert set(dir(bound)) == {_snake(op) for op in _operations(generated)}, group
        for op in _operations(generated):
            function = getattr(bound, _snake(op))
            assert inspect.iscoroutinefunction(function)
            assert function.__wrapped__ is getattr(generated, f"{op}_async")


def test_async_manager_sends_its_token(stub, secrets) -> None:
    script(stub, {
        "expect": {"method": "POST", "path": "/v1/commands", "headers": {"authorization": f"Bearer {secrets['managerToken']}"}},
        "respond": {"json": {"commandId": "cmd_1", "inlineAllowedUpTo": 1024, "next": "poll", "state": "PENDING"}},
    })
    request = manager.CreateCommandRequest(
        command="generate-report", deploymentId="dep_1", params=manager.BodySpecInline(mode="inline", inlineBase64="e30="))
    created = asyncio.run(AsyncAlienManager(stub.base, secrets["managerToken"]).commands.create_command(request))
    assert created.commandId == "cmd_1"
    assert repr(AsyncAlienManager(stub.base, secrets["managerToken"])) == f"AsyncAlienManager(server_url={stub.base!r})"
    requests(stub)


@pytest.mark.parametrize("service", ["platform", "manager"])
def test_every_operation_of_alien_typescript_sdks_is_bound(secrets, service: str) -> None:
    """Every operation of Alien's TypeScript SDKs (the naming oracle, extracted
    from their sources) is a method under its group and name in snake_case; a
    Python keyword gets a trailing `_` (`deployments.import_`)."""
    oracle = json.loads((Path(__file__).parents[3] / "tools/openapi2baml/tests/fixtures/naming-oracle.json").read_text())
    client = Alien(secrets["apiKey"], server_url="http://127.0.0.1:1") if service == "platform" else AlienManager("http://127.0.0.1:1")
    entries = [e for e in oracle["operations"][service].values() if e["in_sdk"]]
    assert len(entries) > 50
    missing = []
    for e in entries:
        target = client if e["group"] is None else getattr(client, _snake(e["group"]))
        name = _snake(e["method_name"])
        if not callable(getattr(target, name + "_" if keyword.iskeyword(name) else name, None)):
            missing.append(f"{e['group']}.{e['method_name']}")
    assert missing == []


def test_bound_names() -> None:
    alien = Alien("key", server_url="http://127.0.0.1:1")
    assert alien.deployment_groups.create_token.__wrapped__ is importlib.import_module("baml_sdk.platform.deploymentGroups").createToken
    assert AlienManager("http://127.0.0.1:1").commands.create_command.__wrapped__ is importlib.import_module("baml_sdk.manager.commands").createCommand
    with pytest.raises(AttributeError, match="createToken"):
        alien.deployment_groups.createToken  # noqa: B018  (only snake_case names are bound)


def test_snake_case_names() -> None:
    assert _snake("createToken") == "create_token"
    assert _snake("revokeAPIKey") == "revoke_api_key"
    assert _snake("createExternalAIModelCheck") == "create_external_ai_model_check"
    assert _snake("getGcpOAuthProvider") == "get_gcp_oauth_provider"  # OAuth is one word


def test_shutdown(tmp_path) -> None:
    program = textwrap.dedent("""
        import alien_sdk
        from baml_bridge import BamlPanic
        alien_sdk.shutdown()
        alien_sdk.shutdown()
        try:
            alien_sdk.cloudformation_launch_url(alien_sdk.CloudFormationLaunch(
                templateUrl="https://example.com/setup.yaml", region="us-east-1", stackName="s", parameters={}))
        except BamlPanic:
            print("BamlPanic after shutdown")
    """)
    home = tmp_path / "baml_home"
    home.mkdir()
    env = {k: v for k, v in os.environ.items() if not k.startswith("BOUNDARY_")}
    env.update(BAML_HOME=str(home), ALIEN_TELEMETRY="off")
    run = subprocess.run([sys.executable, "-c", program], cwd=Path(alien_sdk.__file__).parents[1],
                         env=env, capture_output=True, text=True, timeout=60)
    assert (run.returncode, run.stdout.strip()) == (0, "BamlPanic after shutdown"), run.stderr
    assert list(home.rglob("*")) == []
