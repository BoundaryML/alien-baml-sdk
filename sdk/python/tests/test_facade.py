"""The facade itself: client options, what is bound, the error type, shutdown.

Each call goes to conformance/stub-server, scripted by the test.
"""

from __future__ import annotations

import ast
import asyncio
import copy
import gc
import importlib
import inspect
import json
import keyword
import os
import pickle
import pkgutil
import re
import subprocess
import sys
import textwrap
import traceback
import types
from datetime import timedelta
from fractions import Fraction
from pathlib import Path
from typing import Any, Callable

import pytest
from baml_bridge import UNSET as BAML_UNSET
from baml_bridge import BamlCancelledError, BamlError

import alien_sdk
from alien_sdk import Alien, AlienError, AlienManager, AsyncAlien, AsyncAlienManager, _gen, manager, platform
from alien_sdk._errors import _acall, _call


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


CLIENTS = [Alien, AsyncAlien, AlienManager, AsyncAlienManager]


def new_client(cls: type, **options):
    """A client of any of the four classes; nothing is called."""
    if cls in (Alien, AsyncAlien):
        return cls("key", **options)
    return cls("http://127.0.0.1:9", **options)


@pytest.mark.parametrize("cls", CLIENTS)
@pytest.mark.parametrize(("timeout", "timeout_ms"), [
    (30, 30_000),
    (30.0, 30_000),
    (1.5, 1_500),
    (2.007, 2_007),  # 2.007 * 1000 is 2007.0000000000002
    (0.1, 100),  # the float is a little over 0.1
    (0.0015, 2),  # rounded up
    (0.0010000001, 2),
    (0.0001, 1),  # a sub-millisecond timeout still limits
    (1e-12, 1),
    (Fraction(3, 10), 300),  # any real number
    (1e30, 2**62 - 1),  # the largest BAML int
    (10**400, 2**62 - 1),  # too large for a float
    (None, 0),  # no limit
])
def test_timeout_is_seconds(cls, timeout, timeout_ms) -> None:
    assert new_client(cls, timeout=timeout)._client.timeout_ms == timeout_ms


@pytest.mark.parametrize("cls", CLIENTS)
def test_timeout_defaults_to_five_minutes(cls) -> None:
    assert alien_sdk.DEFAULT_TIMEOUT == 300.0
    assert new_client(cls)._client.timeout_ms == 300_000


@pytest.mark.parametrize("cls", CLIENTS)
@pytest.mark.parametrize(("timeout", "error", "message"), [
    (0, ValueError, "not 0; pass None for no limit"),
    (0.0, ValueError, "not 0.0; pass None for no limit"),
    (-1, ValueError, "not -1; pass None for no limit"),
    (float("nan"), ValueError, "not nan; pass None for no limit"),
    (float("inf"), ValueError, "not inf; pass None for no limit"),
    (True, TypeError, "not bool"),
    ("30", TypeError, "not str"),
    (timedelta(seconds=30), TypeError, "not timedelta; pass .total_seconds()"),
])
def test_invalid_timeout_fails_at_construction(cls, timeout, error, message) -> None:
    with pytest.raises(error, match=re.escape(message)):
        new_client(cls, timeout=timeout)


@pytest.mark.parametrize("cls", CLIENTS)
def test_timeout_ms_is_not_an_option(cls) -> None:
    with pytest.raises(TypeError, match="unexpected keyword argument 'timeout_ms'"):
        new_client(cls, timeout_ms=300)


MANAGER = {"id": "mgr_1", "name": "primary", "targets": ["aws"], "managementConfigs": {}, "isSystem": False,
           "workspaceId": "ws_1", "status": "healthy", "managedDeploymentCount": 0, "defaultProjectCount": 0,
           "createdAt": "2026-10-01T00:00:00Z"}


def test_no_timeout_waits(stub, secrets) -> None:
    # None reaches BAML as 0 ms, which must mean no limit, not "time out at once".
    script(stub, {"expect": {"method": "GET", "path": "/v1/managers/mgr_1"}, "respond": {"json": MANAGER, "delay_ms": 1000}})
    assert Alien(secrets["apiKey"], server_url=stub.base, timeout=None).managers.get("mgr_1").id == "mgr_1"
    requests(stub)


def test_per_call_timeout_is_milliseconds(stub, secrets) -> None:
    # BAML's per-call option is in milliseconds, and the shorter limit ends the call.
    script(stub, {"expect": {"method": "GET", "path": "/v1/managers/mgr_1"}, "respond": {"json": MANAGER, "delay_ms": 3000}})
    alien = Alien(secrets["apiKey"], server_url=stub.base, timeout=5.0)
    with pytest.raises(BamlCancelledError):
        alien.managers.get("mgr_1", _baml={"timeout_ms": 100})


def test_default_server_url(secrets) -> None:
    assert alien_sdk.DEFAULT_SERVER_URL == "https://api.alien.dev"
    assert repr(Alien(secrets["apiKey"])) == "Alien(server_url='https://api.alien.dev', workspace=None)"
    # None means the default, as an unset serverURL does in the other SDKs.
    assert repr(Alien(secrets["apiKey"], server_url=None)) == "Alien(server_url='https://api.alien.dev', workspace=None)"
    assert repr(AsyncAlien(secrets["apiKey"])) == "AsyncAlien(server_url='https://api.alien.dev', workspace=None)"


def test_keyword_arguments_and_fields_are_snake_case(stub, secrets) -> None:
    alien = Alien(secrets["apiKey"], server_url=stub.base, workspace="acme")
    script(stub, {
        "expect": {"method": "GET", "path": "/v1/deployments?workspace=acme&project=web&deploymentGroup=prod&limit=2"},
        "respond": {"json": {"items": [], "nextCursor": "c2"}},
    })
    page = alien.deployments.list(project="web", deployment_group="prod", limit=2)
    assert page.next_cursor == "c2"
    # Here the pydantic alias is the wire name (docs/gaps.md, B27, for where it is not).
    assert page.model_dump(by_alias=True) == {"items": [], "nextCursor": "c2"}
    requests(stub)
    # The docstring names each argument by the keyword the method takes.
    doc = alien.deployments.list.__doc__ or ""
    assert "- `deployment_group`:" in doc and "`deploymentGroup`:" not in doc


def test_a_wrong_keyword_raises_type_error(stub, secrets) -> None:
    """The methods have the generated signatures, so Python itself rejects a
    keyword they do not take, before a request: a misspelling, and the BAML
    name (the API's camelCase) as well (docs/api-shape.md)."""
    alien = Alien(secrets["apiKey"], server_url=stub.base, workspace="acme")
    async_alien = AsyncAlien(secrets["apiKey"], server_url=stub.base, workspace="acme")
    script(stub)
    for wrong in ("deployment_grop", "deploymentGroup"):
        with pytest.raises(TypeError, match=f"unexpected keyword argument '{wrong}'"):
            alien.deployments.list(**{wrong: "prod"})
        with pytest.raises(TypeError, match=f"unexpected keyword argument '{wrong}'"):
            async_alien.deployments.list(**{wrong: "prod"})  # raises before a coroutine exists
    with pytest.raises(TypeError, match="missing 1 required positional argument: 'id'"):
        alien.managers.get()
    with pytest.raises(TypeError, match="takes 2 positional arguments but 3 were given"):
        alien.managers.get("mgr_1", "extra")
    with pytest.raises(TypeError, match="unexpected keyword argument 'commandId'"):
        AlienManager(stub.base).commands.get_command_status("cmd_1", commandId="cmd_1")
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
        command="generate-report", deployment_id="dep_1", params=manager.BodySpecInline(mode="inline", inline_base64="e30=")))
    assert created.command_id == "cmd_1"
    requests(stub)


@pytest.mark.parametrize("token", [None, ""])
def test_manager_without_token_sends_no_authorization(stub, token) -> None:
    script(stub, {
        "expect": {"method": "POST", "path": "/v1/commands", "headers_absent": ["authorization"]},
        "respond": {"json": {"commandId": "cmd_1", "inlineAllowedUpTo": 1024, "next": "poll", "state": "PENDING"}},
    })
    AlienManager(stub.base, token).commands.create_command(manager.CreateCommandRequest(
        command="c", deployment_id="d", params=manager.BodySpecInline(mode="inline", inline_base64="")))
    requests(stub)


def _not_found(stub, secrets, cls: type[Alien | AsyncAlien] = Alien) -> AlienError:
    script(stub, {
        "expect": {"method": "GET", "path": "/v1/managers/mgr_x?workspace=acme"},
        "respond": {"status": 404, "json": {
            "code": "NOT_FOUND", "message": "Manager not found.", "retryable": False, "httpStatusCode": 404,
            "hint": "Check the ID.", "requestId": "req_1", "context": {"id": "mgr_x"},
        }},
    })
    with pytest.raises(AlienError) as info:
        result = cls(secrets["apiKey"], server_url=stub.base, workspace="acme").managers.get("mgr_x")
        if inspect.iscoroutine(result):
            asyncio.run(result)
    requests(stub)
    return info.value


# Every field of alien_sdk.AlienError, in the order its constructor takes them.
ERROR_FIELDS = ("code", "message", "retryable", "http_status", "context", "hint", "request_id", "source", "internal")


def _fields(error: AlienError | None) -> dict | None:
    """Every field of an error, ``source`` level by level."""
    if error is None:
        return None
    return {f: _fields(error.source) if f == "source" else getattr(error, f) for f in ERROR_FIELDS}


def test_alien_error(stub, secrets) -> None:
    error = _not_found(stub, secrets)
    assert (error.code, error.retryable, error.http_status) == ("NOT_FOUND", False, 404)
    assert str(error) == "NOT_FOUND: Manager not found."
    assert (error.hint, error.request_id, error.internal, error.source) == ("Check the ID.", "req_1", None, None)
    # The server's context as sent: no path, request ID, hint or details added.
    assert error.context == {"id": "mgr_x"}
    # No source, so no cause; and the runtime's error is not chained either.
    assert error.__cause__ is None and error.__context__ is None and error.__suppress_context__ is False
    assert f"{type(error).__module__}.{type(error).__qualname__}" == "alien_sdk.AlienError"
    for clone in (pickle.loads(pickle.dumps(error)), copy.copy(error), copy.deepcopy(error)):
        assert type(clone) is AlienError
        assert _fields(clone) == _fields(error)
        assert clone.__cause__ is None


def _raw_error(stub, secrets, body: str, asynchronous: bool = False) -> AlienError:
    """The error for this response body, sent as written (JSON text)."""
    script(stub, {"expect": {"method": "GET", "path": "/v1/managers/mgr_x"},
                  "respond": {"status": 400, "text": body, "headers": {"content-type": "application/json"}}})
    with pytest.raises(AlienError) as info:
        if asynchronous:
            asyncio.run(AsyncAlien(secrets["apiKey"], server_url=stub.base).managers.get("mgr_x"))
        else:
            Alien(secrets["apiKey"], server_url=stub.base).managers.get("mgr_x")
    requests(stub)
    return info.value


def _typed(value: object) -> object:
    """A JSON value with the type of every number, bool and None spelled out,
    so that 1, 1.0 and True, which Python compares equal, differ."""
    if isinstance(value, dict):
        return {key: _typed(member) for key, member in value.items()}
    if isinstance(value, list):
        return [_typed(member) for member in value]
    return (type(value).__name__, value)


@pytest.mark.parametrize("asynchronous", [False, True])
def test_context_is_the_servers_json(stub, secrets, asynchronous: bool) -> None:
    nested = ('{"s":"t","i":7,"neg":-7,"f":2.5,"whole":1.0,"zero":0,"b":true,"n":null,'
              '"past_2_53":9007199254740993,"huge":123456789012345678901234567890,'
              '"arr":[1,"a",null,{"k":[2.5,-0.125]}],"obj":{"z":1,"a":{"deep":[]}}}')
    error = _raw_error(stub, secrets, (
        f'{{"code":"TOP","message":"m","context":{nested},'
        f'"source":{{"code":"L1","message":"m","context":[1,{nested}],'
        '"source":{"code":"L2","message":"m","context":"just text",'
        '"source":{"code":"L3","message":"m","context":-0.5,'
        '"source":{"code":"L4","message":"m","context":7,'
        '"source":{"code":"L5","message":"m","context":false}}}}}}'
    ), asynchronous)
    want = {
        "s": "t", "i": 7, "neg": -7, "f": 2.5, "whole": 1.0, "zero": 0, "b": True, "n": None,
        # Python's int has no limit: integers of any size arrive exact.
        "past_2_53": 9007199254740993, "huge": 123456789012345678901234567890,
        "arr": [1, "a", None, {"k": [2.5, -0.125]}], "obj": {"z": 1, "a": {"deep": []}},
    }
    levels = []
    level: AlienError | None = error
    while level is not None:
        levels.append(level.context)
        level = level.source
    # dict, list, str, int, float, bool and None: json.loads' types, an integer an int and 1.0 a float.
    assert [_typed(context) for context in levels] == [_typed(c) for c in (want, [1, want], "just text", -0.5, 7, False)]
    assert json.loads(json.dumps(error.context)) == want
    assert list(error.context) == list(want)  # keys in the order sent
    # Stored as given: the constructor does not copy or convert it.
    given = {"a": [1]}
    assert AlienError("X", "m", False, context=given).context is given
    for scalar in ("text", 7, 2.5, True, [1, 2]):
        assert AlienError("X", "m", False, None, scalar).context == scalar


def test_a_context_is_never_the_request_path_or_the_response_body(stub, secrets) -> None:
    # A server error body that is not the API's: nothing to put in the context.
    script(stub, {"expect": {"method": "GET", "path": "/v1/managers/mgr_x"}, "respond": {"status": 502, "text": "bad gateway: secret"}})
    with pytest.raises(AlienError) as info:
        Alien(secrets["apiKey"], server_url=stub.base).managers.get("mgr_x")
    assert (info.value.code, info.value.context) == ("PLATFORM_HTTP_ERROR", None)
    # JSON of another shape: where and why it is not the API's error body.
    script(stub, {"expect": {"method": "GET", "path": "/v1/managers/mgr_x"}, "respond": {"status": 400, "json": {"code": 7, "message": "m"}}})
    with pytest.raises(AlienError) as info:
        Alien(secrets["apiKey"], server_url=stub.base).managers.get("mgr_x")
    assert (info.value.code, info.value.context) == ("PLATFORM_HTTP_ERROR", {"json_path": ".code", "decode_error": "expected string"})
    # A request that cannot be sent: the method, not the path.
    with pytest.raises(AlienError) as info:
        Alien(secrets["apiKey"], server_url="http://127.0.0.1:1").managers.get("mgr_x")
    assert (info.value.code, info.value.context) == ("COMMUNICATION_ERROR", {"method": "GET"})
    assert "/v1/managers/mgr_x" in info.value.message  # the message still names it
    requests(stub)


# Every field of Alien's error, with a two-level `source` chain.
CHAINED = {
    "code": "DEPLOY_FAILED", "message": "deploy failed", "retryable": False, "internal": True,
    "hint": "see the source", "requestId": "req_top", "context": {"step": 3},
    "source": {
        "code": "SANDBOX_REFUSED", "message": "the sandbox refused", "retryable": True, "internal": False,
        "hint": "ask an admin", "requestId": "req_inner", "httpStatusCode": 403, "context": {"policy": "p1"},
        "source": {"code": "QUOTA", "message": "quota exceeded", "internal": True},
    },
}


def _chained(stub, secrets, asynchronous: bool = False) -> AlienError:
    script(stub, {"expect": {"method": "GET", "path": "/v1/managers/mgr_x"}, "respond": {"status": 500, "json": CHAINED}})
    with pytest.raises(AlienError) as info:
        if asynchronous:
            asyncio.run(AsyncAlien(secrets["apiKey"], server_url=stub.base).managers.get("mgr_x"))
        else:
            Alien(secrets["apiKey"], server_url=stub.base).managers.get("mgr_x")
    requests(stub)
    return info.value


@pytest.mark.parametrize("asynchronous", [False, True])
def test_the_cause_chain_is_the_servers_source_chain(stub, secrets, asynchronous: bool) -> None:
    error = _chained(stub, secrets, asynchronous)
    assert _fields(error) == {
        "code": "DEPLOY_FAILED", "message": "deploy failed", "retryable": False, "http_status": 500,
        "context": {"step": 3},
        "hint": "see the source", "request_id": "req_top", "internal": True,
        "source": {
            "code": "SANDBOX_REFUSED", "message": "the sandbox refused", "retryable": True, "http_status": 403,
            "context": {"policy": "p1"}, "hint": "ask an admin", "request_id": "req_inner", "internal": False,
            "source": {
                "code": "QUOTA", "message": "quota exceeded", "retryable": False, "http_status": None,
                "context": None, "hint": None, "request_id": None, "internal": True, "source": None,
            },
        },
    }
    first = error.source
    assert isinstance(first, AlienError) and isinstance(first.source, AlienError)
    assert error.__cause__ is first and error.__suppress_context__ is True
    assert first.__cause__ is first.source and first.__suppress_context__ is True
    assert first.source.__cause__ is None and first.source.source is None
    # The chain is only the server's: no runtime error at any level, as cause or as context.
    level: BaseException | None = error
    while level is not None:
        assert type(level) is AlienError and level.__context__ is None
        level = level.__cause__
    printed = "".join(traceback.format_exception(error))
    assert printed.index("QUOTA: quota exceeded") < printed.index("SANDBOX_REFUSED: the sandbox refused") < printed.index("DEPLOY_FAILED: deploy failed")
    assert printed.count("direct cause of the following exception") == 2 and "BamlError" not in printed


def test_a_chained_error_survives_pickle_and_copy(stub, secrets) -> None:
    error = _chained(stub, secrets)
    for clone in (pickle.loads(pickle.dumps(error)), copy.copy(error), copy.deepcopy(error)):
        assert type(clone) is AlienError
        assert _fields(clone) == _fields(error)
        assert type(clone.source) is AlienError and type(clone.source.source) is AlienError
        # __init__ sets the cause, so a rebuilt error is chained as the raised one was.
        assert clone.__cause__ is clone.source and clone.__suppress_context__ is True
        assert clone.source.__cause__ is clone.source.source
    # So is an error built directly, with `source` by position or by keyword.
    inner = AlienError("INNER", "inner", False)
    by_keyword = AlienError("OUTER", "outer", True, source=inner)
    by_position = AlienError("OUTER", "outer", True, 502, {"field": "name"}, "a hint", "req_1", inner, False)
    assert by_keyword.__cause__ is inner and by_position.__cause__ is inner
    assert (by_position.hint, by_position.request_id, by_position.source, by_position.internal) == ("a hint", "req_1", inner, False)
    assert inner.__cause__ is None and inner.__suppress_context__ is False


def test_the_facade_maps_every_field_of_the_generated_error() -> None:
    """A field BAML's AlienError gains fails here until the facade maps it:
    the generated class, the facade's constructor and its attributes have the
    same names, and the conversion carries each one at every level."""
    generated = set(_gen.errors.AlienError.model_fields)
    assert generated == set(ERROR_FIELDS)
    parameters = list(inspect.signature(AlienError.__init__).parameters.values())[1:]
    assert tuple(p.name for p in parameters) == ERROR_FIELDS
    assert all(p.kind is inspect.Parameter.POSITIONAL_OR_KEYWORD for p in parameters)  # pickle rebuilds with cls(*args)
    innermost = _gen.errors.AlienError(
        code="INNER", message="inner", retryable=True, http_status=403, context=["inner", 1, {"a": None}],
        hint="inner hint", request_id="req_inner", source=None, internal=True)
    value = _gen.errors.AlienError(
        code="OUTER", message="outer", retryable=False, http_status=500, context={"field": "name", "limits": [1, 2.5]},
        hint="outer hint", request_id="req_outer", source=innermost, internal=False)
    assert set(value.model_fields_set) == generated  # the fixture sets every field
    error = alien_sdk._errors._from_generated(value)
    assert _fields(error) == value.model_dump()
    assert set(vars(error)) == generated
    assert error.args == tuple(getattr(error, f) for f in ERROR_FIELDS)


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
    for binding in deployment.stack_settings.external_bindings.values():
        match binding:
            case platform.ExternalBindingOther(raw=raw):
                got.append(raw["service"])
            case platform.ExternalBindingStorageS3(bucket_name=bucket):
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


def _chain(error: BaseException) -> list[BaseException]:
    """An exception and every exception reachable from it through
    ``__cause__`` and ``__context__``, whatever ``__suppress_context__`` says:
    error reporters read both."""
    found: list[BaseException] = []
    pending: list[BaseException | None] = [error]
    while pending:
        level = pending.pop()
        if level is None or any(level is seen for seen in found):
            continue
        found.append(level)
        pending += [level.__cause__, level.__context__]
    return found


def _holds(value: object, secret: str, seen: set[int], depth: int = 8) -> bool:
    """Whether ``value`` holds ``secret`` by reference: it is a string or
    bytes containing it, or a container, a pydantic model or another object
    with attributes that leads to one, a few levels down."""
    if isinstance(value, str):
        return secret in value
    if isinstance(value, (bytes, bytearray)) and secret.encode() in bytes(value):
        return True
    if depth == 0 or id(value) in seen or isinstance(value, (type, types.ModuleType, types.FunctionType, types.MethodType)):
        return False
    seen.add(id(value))
    children: list[object] = []
    if isinstance(value, dict):
        children += [*value.keys(), *value.values()]
    elif isinstance(value, (list, tuple, set, frozenset)):
        children += value
    attributes = getattr(value, "__dict__", None)
    if isinstance(attributes, dict):
        children += attributes.values()
    return any(_holds(child, secret, seen, depth - 1) for child in children)


def _locals_holding(error: BaseException, secret: str, by_reference: bool = False) -> set[str]:
    """The frame locals, in the tracebacks of ``error`` and of everything
    chained to it, that hold ``secret``; this file's own frames aside. A
    local holds it when its repr shows it, which is what error reporters
    record. With ``by_reference``, also when it leads to it (:func:`_holds`):
    then the facade's own frames are left out too, since a method's ``self``
    is the client, which has the key."""
    holding = set()
    for level in _chain(error):
        for frame, _ in traceback.walk_tb(level.__traceback__):
            directory = Path(frame.f_code.co_filename).parent
            if directory == Path(__file__).parent or (by_reference and directory.name == "alien_sdk"):
                continue
            for name, value in frame.f_locals.items():
                if secret in repr(value) or (by_reference and _holds(value, secret, set())):
                    holding.add(f"{directory.name}.{frame.f_code.co_name}:{name}")
    return holding


def test_the_bridge_keeps_a_calls_arguments_in_its_frame(stub, secrets) -> None:
    """Why an AlienError must not lead to the runtime's error
    (docs/gaps.md#b23), and a check that :func:`_locals_holding` sees it: the
    bridge's frame keeps the call's arguments, the client among them. A
    generated client's repr shows the key; the facade's client hides it from
    repr, but the object is still there."""
    script(stub, *[{"expect": {"method": "GET", "path": "/v1/managers/mgr_x"},
                    "respond": {"status": 404, "json": {"code": "NOT_FOUND", "message": "Manager not found."}}}] * 2)
    key = secrets["apiKey"]

    def in_the_bridge(holding: set[str]) -> bool:
        return any(name.startswith("baml_bridge.") for name in holding)

    with pytest.raises(BamlError) as shown:
        _gen.platform.managers.get_sync(_gen.platform.PlatformClient(base_url=stub.base, api_key=key), "mgr_x")
    assert in_the_bridge(_locals_holding(shown.value, key))
    with pytest.raises(BamlError) as hidden:
        _gen.platform.managers.get_sync(Alien(key, server_url=stub.base)._client, "mgr_x")
    assert _locals_holding(hidden.value, key) == set()
    assert in_the_bridge(_locals_holding(hidden.value, key, by_reference=True))
    requests(stub)


@pytest.mark.parametrize("asynchronous", [False, True])
@pytest.mark.parametrize("failure", ["not_found", "chained"])
def test_credentials_stay_out_of_tracebacks(stub, secrets, failure: str, asynchronous: bool) -> None:
    """Tracebacks that capture locals, as error reporters do. baml_bridge keeps
    a call's arguments, the client with its API key among them, in locals of
    its own frame (docs/gaps.md#b23): an AlienError must not lead there,
    through its traceback, its ``__cause__`` or its ``__context__``, at any
    depth."""
    gc.collect()
    before = {id(o) for o in gc.get_objects() if isinstance(o, BamlError)}
    if failure == "chained":
        error = _chained(stub, secrets, asynchronous)
    elif asynchronous:
        error = _not_found(stub, secrets, AsyncAlien)
    else:
        error = _not_found(stub, secrets)
    assert [type(level) for level in _chain(error)] == [AlienError] * (3 if failure == "chained" else 1)
    # No local of any frame shows the key, and outside the facade's own frames none holds it at all.
    assert _locals_holding(error, secrets["apiKey"]) == set()
    assert _locals_holding(error, secrets["apiKey"], by_reference=True) == set()
    # The frames the traceback does have are the facade's, not the bridge's or the generated SDK's.
    frames = [frame for frame in traceback.extract_tb(error.__traceback__) if Path(frame.filename).parent != Path(__file__).parent]
    assert {Path(frame.filename).parent.name for frame in frames} - {"asyncio"} == {"alien_sdk"}
    assert frames[-1].name == ("_acall" if asynchronous else "_call")
    # And nothing the AlienError references keeps the runtime's error, or its
    # traceback, alive: with the AlienError still held, that error is gone.
    gc.collect()
    assert [o for o in gc.get_objects() if isinstance(o, BamlError) and id(o) not in before] == []


def test_a_setup_helper_error_is_not_chained_to_the_runtimes(secrets) -> None:
    for call in (
        lambda: alien_sdk.cloudformation_launch_url(alien_sdk.CloudFormationLaunch(
            template_url="http://not-https", region="us-east-1", stack_name="s", parameters={})),
        lambda: Alien(secrets["apiKey"], server_url="http://127.0.0.1:1").setup.launch_url_for_project("p", "nowhere", "s"),
        lambda: asyncio.run(AsyncAlien(secrets["apiKey"], server_url="http://127.0.0.1:1").setup.launch_url_for_project("p", "nowhere", "s")),
    ):
        with pytest.raises(AlienError) as info:
            call()
        assert [type(level) for level in _chain(info.value)] == [AlienError]
        assert _locals_holding(info.value, secrets["apiKey"]) == set()


def test_only_a_thrown_alien_error_is_converted() -> None:
    # Anything else propagates as the object it is: a BamlError holding another
    # value, and an error that is not the runtime's.
    for raised in (BamlError({"code": "X"}, class_name="user.other.Error"), ValueError("plain")):
        def fail() -> None:
            raise raised

        async def afail() -> None:
            raise raised

        with pytest.raises(type(raised)) as info:
            _call(fail)
        assert info.value is raised
        with pytest.raises(type(raised)) as info:
            asyncio.run(_acall(afail))
        assert info.value is raised
    assert _call(lambda a, *, b: (a, b), 1, b=2) == (1, 2)

    async def add(a: int, *, b: int) -> int:
        return a + b

    assert asyncio.run(_acall(add, 1, b=2)) == 3


def _operations(module: types.ModuleType) -> set[str]:
    """A generated module's operations: each async function with a blocking ``_sync`` twin."""
    return {
        name for name, value in vars(module).items()
        if isinstance(value, types.FunctionType) and value.__module__ == module.__name__
        and not name.startswith("_") and isinstance(vars(module).get(name + "_sync"), types.FunctionType)
    }


def _public(value: object) -> set[str]:
    return {name for name in dir(value) if not name.startswith("_")}


def _stub_signatures(module: types.ModuleType) -> dict[str, list[tuple[str, inspect._ParameterKind, bool]]]:
    """Each generated operation's parameters after the client, as its stub
    (``__init__.pyi``) declares them: name, kind and whether it has a default."""
    tree = ast.parse((Path(module.__file__).parent / "__init__.pyi").read_text(encoding="utf-8"))
    out = {}
    for node in tree.body:
        if isinstance(node, ast.AsyncFunctionDef):
            a = node.args
            out[node.name] = [
                *((arg.arg, inspect.Parameter.POSITIONAL_OR_KEYWORD, False) for arg in a.args[1:]),
                *((arg.arg, inspect.Parameter.KEYWORD_ONLY, True) for arg in a.kwonlyargs),
            ]
    return out


def _doc_by_keyword(module: types.ModuleType, op: str) -> str | None:
    """The generated operation's docs with each argument (``- `deploymentGroup`:``)
    named by the Python name the method takes (``- `deployment_group`:``), from
    the ``param_aliases`` the generated module passes ``_define_function``."""
    doc = inspect.getdoc(getattr(module, op))
    tree = ast.parse(Path(module.__file__).read_text(encoding="utf-8"))
    for node in tree.body:
        if (isinstance(node, ast.Assign) and isinstance(node.value, ast.Call)
                and [t.id for t in node.targets if isinstance(t, ast.Name)] == [op]):
            aliases = [ast.literal_eval(kw.value) for kw in node.value.keywords if kw.arg == "param_aliases"]
            python_names = {baml: python for python, baml in (aliases[0] if aliases else {}).items()}
            if doc is not None:
                doc = re.sub(r"^- `([^`]+)`:", lambda m: f"- `{python_names.get(m[1], m[1])}`:", doc, flags=re.M)
    return doc


@pytest.mark.parametrize("asynchronous", [False, True], ids=["blocking", "async"])
@pytest.mark.parametrize("module", [_gen.platform, _gen.manager], ids=lambda m: m.__name__)
def test_every_generated_operation_is_bound(secrets, monkeypatch, module: types.ModuleType, asynchronous: bool) -> None:
    """Walks every generated operation: the client (``Alien``, ``AsyncAlien``,
    ``AlienManager``, ``AsyncAlienManager``) has a method for it under its
    group, with the stub's parameters (without the client) and docs (each
    argument named by its keyword), and the
    method calls the generated function (``<name>_sync`` when blocking,
    ``<name>`` when async) with the client and its arguments, leaving each
    optional one UNSET (not passed)."""
    if module is _gen.platform:
        client: Any = (AsyncAlien if asynchronous else Alien)(secrets["apiKey"], server_url="http://127.0.0.1:1")
    else:
        client = (AsyncAlienManager if asynchronous else AlienManager)("http://127.0.0.1:1")
    groups = {info.name for info in pkgutil.iter_modules(module.__path__) if info.ispkg}
    assert groups
    own = {"setup"} if module is _gen.platform else set()
    assert not own & groups, "a generated group is shadowed by a facade attribute"
    assert _public(client) == groups | _operations(module) | own
    calls: list[tuple[str, tuple, dict]] = []

    def recorder(target: str) -> Callable[..., Any]:
        def record(*args: Any, **kwargs: Any) -> str:
            calls.append((target, args, kwargs))
            return target

        async def record_async(*args: Any, **kwargs: Any) -> str:
            return record(*args, **kwargs)

        return record_async if asynchronous else record

    bound = 0
    for owner, generated in [(client, module), *((getattr(client, g), importlib.import_module(f"{module.__name__}.{g}")) for g in groups)]:
        if owner is not client:
            assert _public(owner) == _operations(generated), generated.__name__
        stubs = _stub_signatures(generated)
        for op in _operations(generated):
            method = getattr(owner, op)
            where = f"{generated.__name__}.{op}"
            assert inspect.iscoroutinefunction(method) is asynchronous, where
            parameters = list(inspect.signature(method).parameters.values())
            assert [(p.name, p.kind, p.default is not p.empty) for p in parameters] == stubs[op], where
            assert inspect.getdoc(method) == _doc_by_keyword(generated, op), where
            target = op if asynchronous else f"{op}_sync"
            monkeypatch.setattr(generated, target, recorder(f"{generated.__name__}.{target}"))
            required = [object() for p in parameters if p.default is p.empty]
            result = asyncio.run(method(*required)) if asynchronous else method(*required)
            assert result == f"{generated.__name__}.{target}", where
            _, args, kwargs = calls.pop()
            assert args[0] is client._client and list(args[1:]) == required, where
            assert kwargs == {p.name: (None if p.name == "_baml" else BAML_UNSET) for p in parameters if p.kind is p.KEYWORD_ONLY}, where
            bound += 1
    assert bound == (279 if module is _gen.platform else 53)


def test_a_set_keyword_reaches_the_generated_function(monkeypatch) -> None:
    deployments = importlib.import_module(f"{_gen.platform.__name__}.deployments")
    seen = {}
    monkeypatch.setattr(deployments, "list_sync", lambda client, **kwargs: seen.update(kwargs))
    Alien("key", server_url="http://127.0.0.1:1").deployments.list(deployment_group="prod", limit=None, _baml={"timeout_ms": 5})
    assert seen["deployment_group"] == "prod" and seen["limit"] is None and seen["_baml"] == {"timeout_ms": 5}
    assert seen["project"] is BAML_UNSET


def test_type_checkers_see_the_signatures() -> None:
    """mypy (strict on the facade) checks tests/typed_usage.py: calls with the
    generated signatures type-check, and each wrong call it marks `type:
    ignore` must be an error (--warn-unused-ignores), which it is only when
    mypy sees the real signatures (not Any)."""
    root = Path(__file__).parents[1]
    # Installed (a wheel), the package needs the marker for type checkers to read it (PEP 561).
    assert (root / "alien_sdk" / "py.typed").is_file()
    result = subprocess.run(
        [sys.executable, "-m", "mypy", "--strict", "--warn-unused-ignores", "--no-incremental", "--cache-dir", os.devnull,
         "tests/typed_usage.py", "alien_sdk"],
        cwd=root, capture_output=True, text=True, timeout=600,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    revealed = re.findall(r'typed_usage\.py:\d+: note: Revealed type is "(.*)"', result.stdout)
    assert len(revealed) == 2, result.stdout
    blocking, asynchronous = revealed
    assert blocking.startswith("def (*, project: str | None | UNSET =, deployment_group: str | None | UNSET =")
    assert blocking.endswith(f"-> {_gen.platform.__name__}.ListDeploymentsResponse")
    assert asynchronous.endswith(f"-> typing.Coroutine[Any, Any, {_gen.platform.__name__}.ListDeploymentsResponse]")


def test_async_manager_sends_its_token(stub, secrets) -> None:
    script(stub, {
        "expect": {"method": "POST", "path": "/v1/commands", "headers": {"authorization": f"Bearer {secrets['managerToken']}"}},
        "respond": {"json": {"commandId": "cmd_1", "inlineAllowedUpTo": 1024, "next": "poll", "state": "PENDING"}},
    })
    request = manager.CreateCommandRequest(
        command="generate-report", deployment_id="dep_1", params=manager.BodySpecInline(mode="inline", inline_base64="e30="))
    created = asyncio.run(AsyncAlienManager(stub.base, secrets["managerToken"]).commands.create_command(request))
    assert created.command_id == "cmd_1"
    assert repr(AsyncAlienManager(stub.base, secrets["managerToken"])) == f"AsyncAlienManager(server_url={stub.base!r})"
    requests(stub)


# Words that stay whole in snake_case (BAML #5524): ``getGcpOAuthProvider`` → ``get_gcp_oauth_provider``.
_KNOWN_WORDS = {"OAuth": "Oauth", "GraphQL": "Graphql", "GitHub": "Github"}


def _snake(name: str) -> str:
    """The snake_case Alien asked for: ``createToken`` → ``create_token``,
    ``revokeAPIKey`` → ``revoke_api_key``, ``getGcpOAuthProvider`` →
    ``get_gcp_oauth_provider``. BAML's ``language`` naming spells the
    generated names; this test checks that they read this way."""
    for word, whole in _KNOWN_WORDS.items():
        name = name.replace(word, whole)
    return re.sub(r"(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])", "_", name).lower()


@pytest.mark.parametrize("service", ["platform", "manager"])
def test_every_operation_of_alien_typescript_sdks_is_bound(secrets, service: str) -> None:
    """Every operation of Alien's TypeScript SDKs (the naming oracle, extracted
    from their sources) is a method under its group and name in snake_case; a
    Python keyword gets a trailing `_` (`deployments.import_`)."""
    oracle = json.loads((Path(__file__).parents[3] / "tools/openapi2baml/tests/fixtures/naming-oracle.json").read_text())
    client = Alien(secrets["apiKey"], server_url="http://127.0.0.1:1") if service == "platform" else AlienManager("http://127.0.0.1:1")
    entries = [e for e in oracle["operations"][service].values() if e["in_sdk"]]
    assert len(entries) == (279 if service == "platform" else 53)
    missing = []
    for e in entries:
        target = client if e["group"] is None else getattr(client, _snake(e["group"]))
        name = _snake(e["method_name"])
        if not callable(getattr(target, name + "_" if keyword.iskeyword(name) else name, None)):
            missing.append(f"{e['group']}.{e['method_name']}")
    assert missing == []


def test_bound_names() -> None:
    alien = Alien("key", server_url="http://127.0.0.1:1")
    assert alien.deployment_groups.create_token.__qualname__ == "PlatformDeploymentGroups.create_token"
    assert AsyncAlienManager("http://127.0.0.1:1").commands.create_command.__qualname__ == "AsyncManagerCommands.create_command"
    assert callable(alien.api_keys.revoke) and callable(alien.projects.get_gcp_oauth_provider)
    with pytest.raises(AttributeError, match="createToken"):
        alien.deployment_groups.createToken  # noqa: B018  (only the generated snake_case names are bound)
    with pytest.raises(AttributeError, match="create_token_sync"):
        alien.deployment_groups.create_token_sync  # noqa: B018  (bound as create_token)


def test_shutdown(tmp_path) -> None:
    program = textwrap.dedent("""
        import alien_sdk
        from baml_bridge import BamlPanic
        alien_sdk.shutdown()
        alien_sdk.shutdown()
        try:
            alien_sdk.cloudformation_launch_url(alien_sdk.CloudFormationLaunch(
                template_url="https://example.com/setup.yaml", region="us-east-1", stack_name="s", parameters={}))
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
