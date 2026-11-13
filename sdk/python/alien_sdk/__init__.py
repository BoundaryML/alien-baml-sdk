"""Alien SDK for Python.

A thin facade over the generated BAML SDK (``alien_sdk._gen`` imports it):
it builds clients, renames, converts errors and picks the public surface.
Every behavior lives in the BAML project (``baml/baml_src``).

:mod:`alien_sdk.platform` and :mod:`alien_sdk.manager` hold the request and
response types of each API (generated pydantic models, enums and type
aliases), one namespace per API so their names never clash::

    from alien_sdk.platform import CreateSetupLinkRequest
    request = CreateSetupLinkRequest.model_validate(form)   # camelCase or snake_case keys

They are also what ``isinstance`` checks need: a union member this SDK does
not know is ``platform.EventDataOther`` and the like (docs/api-shape.md).
Call operations through :class:`Alien` and :class:`AlienManager` (or
:class:`AsyncAlien` and :class:`AsyncAlienManager`).
"""

from __future__ import annotations

import datetime as _datetime
import fractions as _fractions
import math as _math
import numbers as _numbers
import os as _os
import typing as _typing

import pydantic as _pydantic
from baml_bridge import shutdown_runtime as _shutdown_runtime

# Importing the generated SDK (through _gen, the one module that names it) starts the runtime.
from . import _gen
from ._gen import setup as _setup

from . import _operations, manager, platform  # (platform, manager: the request and response types)
from ._errors import AlienError, _acall, _call

CloudFormationLaunch = _setup.CloudFormationLaunch

DEFAULT_SERVER_URL = "https://api.alien.dev"

DEFAULT_TIMEOUT = 300.0
"""The default ``timeout``, in seconds: BAML's default of 5 minutes."""

# The largest BAML int: the generated clients hold the timeout as one.
_MAX_TIMEOUT_MS = (1 << 62) - 1


# The generated client classes are pydantic models, whose repr() shows every
# field. These subclasses leave the credential out of repr(), vars() and
# debuggers. baml_bridge keeps a call's arguments, the client among them, in
# locals of its own frame: an AlienError's traceback has no such frame
# (_errors._call), the runtime's own errors still do (docs/gaps.md#b23).
class _PlatformClient(_gen.platform.PlatformClient):
    api_key: str = _pydantic.Field(repr=False)


class _ManagerClient(_gen.manager.ManagerClient):
    token: _typing.Optional[str] = _pydantic.Field(default=None, repr=False)


def _timeout_ms(timeout: float | None) -> int:
    """A ``timeout`` in seconds as the generated clients take it: whole
    milliseconds, rounded up so that a sub-millisecond timeout still limits;
    0 (``None``) is no limit."""
    if timeout is None:
        return 0
    if isinstance(timeout, bool) or not isinstance(timeout, _numbers.Real):
        hint = "; pass .total_seconds()" if isinstance(timeout, _datetime.timedelta) else ""
        raise TypeError(f"timeout must be a number of seconds or None, not {type(timeout).__name__}{hint}")
    # A plain int or float from here on (numpy scalars, Fraction): a numpy
    # int64 would wrap around when multiplied below.
    timeout = int(timeout) if isinstance(timeout, _numbers.Integral) else float(timeout)
    if not 0 < timeout < _math.inf:  # NaN fails too
        raise ValueError(f"timeout must be a positive, finite number of seconds, not {timeout!r}; pass None for no limit")
    if timeout >= _MAX_TIMEOUT_MS / 1000:  # before multiplying, which could overflow
        return _MAX_TIMEOUT_MS
    if isinstance(timeout, int):
        return timeout * 1000
    # Round up the decimal the float was written as (its repr, read exactly),
    # not its binary value: 2.007 * 1000 is 2007.0000000000002 and the float
    # 0.1 is a little over 0.1, but they are 2007 and 100 ms, not 2008 and 101.
    return min(max(_math.ceil(_fractions.Fraction(repr(timeout)) * 1000), 1), _MAX_TIMEOUT_MS)


def _platform_client(api_key: str | None, server_url: str | None, workspace: str | None, timeout: float | None) -> _PlatformClient:
    if api_key is None:
        api_key = _os.environ.get("ALIEN_API_KEY")
    if not api_key:
        raise AlienError("INVALID_REQUEST", "no API key: pass api_key or set ALIEN_API_KEY", False)
    if server_url is None:
        server_url = DEFAULT_SERVER_URL
    return _PlatformClient(base_url=server_url, api_key=api_key, workspace=workspace, timeout_ms=_timeout_ms(timeout))


def _manager_client(server_url: str, token: str | None, timeout: float | None) -> _ManagerClient:
    # An empty token sends no Authorization header.
    return _ManagerClient(base_url=server_url, token=token or None, timeout_ms=_timeout_ms(timeout))


class Alien(_operations.Platform):
    """A Platform API client, configured like Alien's TypeScript SDK
    (``@alienplatform/platform-api``): ``api_key`` defaults to the
    ``ALIEN_API_KEY`` environment variable, ``server_url`` (when ``None``) to
    :data:`DEFAULT_SERVER_URL`, and ``workspace`` is optional for
    workspace-scoped API keys.

    ``timeout`` limits each HTTP request, from connecting to reading the
    whole response (Alien's ``timeoutMs``), in seconds as in httpx:
    :data:`DEFAULT_TIMEOUT` (5 minutes) unless set, ``None`` for no limit,
    rounded up to whole milliseconds. Zero, negative or not finite raises
    ``ValueError``; anything but a real number or ``None`` (a ``bool``, a
    ``timedelta``) raises ``TypeError``. A request that runs out
    raises :class:`AlienError` ``COMMUNICATION_ERROR`` (retryable). A per-call
    ``_baml={"timeout_ms": ...}`` (BAML's own option, so in milliseconds)
    limits the whole call as well: the shorter one ends it.

    ``alien.<group>.<method>(...)`` calls any Platform API operation::

        alien = Alien(workspace="my-workspace")
        m = alien.managers.get("mgr_...")
        page = alien.deployments.list(project="web", deployment_group="prod", limit=10)
        page.next_cursor

    Groups, methods, keywords and fields are snake_case
    (``deployment_group=``, ``page.next_cursor``). Required arguments are
    positional and optional ones are keywords. Every method has the generated
    operation's signature, so editors and type checkers see it, and an
    unknown keyword (``deploymentGroup=``, ``deployment_grop=``) raises
    ``TypeError`` before any request.

    Call :func:`shutdown` before the process ends without running ``atexit``
    handlers (a signal such as SIGTERM, ``os._exit()``).
    :class:`AsyncAlien` is the same client with ``async`` methods.
    """

    def __init__(
        self,
        api_key: str | None = None,
        *,
        server_url: str | None = None,
        workspace: str | None = None,
        timeout: float | None = DEFAULT_TIMEOUT,
    ) -> None:
        client = _platform_client(api_key, server_url, workspace, timeout)
        super().__init__(client)
        self.setup = ProjectSetup(client)

    def __repr__(self) -> str:
        return f"{type(self).__name__}(server_url={self._client.base_url!r}, workspace={self._client.workspace!r})"


class AsyncAlien(_operations.AsyncPlatform):
    """:class:`Alien` with ``async`` methods, for asyncio programs::

        alien = AsyncAlien(workspace="my-workspace")
        m = await alien.managers.get("mgr_...")
    """

    def __init__(
        self,
        api_key: str | None = None,
        *,
        server_url: str | None = None,
        workspace: str | None = None,
        timeout: float | None = DEFAULT_TIMEOUT,
    ) -> None:
        client = _platform_client(api_key, server_url, workspace, timeout)
        super().__init__(client)
        self.setup = AsyncProjectSetup(client)

    def __repr__(self) -> str:
        return f"{type(self).__name__}(server_url={self._client.base_url!r}, workspace={self._client.workspace!r})"


class AlienManager(_operations.Manager):
    """A Manager API client, for example for the ``managerUrl`` and ``token`` from
    ``alien.commands.bootstrap``. ``AlienManager(server_url, token).commands.create_command(...)``
    calls any Manager API operation; the token is sent on every operation
    when set (an empty token sends none). ``timeout`` is :class:`Alien`'s.
    Call :func:`shutdown` before the process ends without running ``atexit``
    handlers (a signal such as SIGTERM, ``os._exit()``).
    """

    def __init__(self, server_url: str, token: str | None = None, *, timeout: float | None = DEFAULT_TIMEOUT) -> None:
        super().__init__(_manager_client(server_url, token, timeout))

    def __repr__(self) -> str:
        return f"{type(self).__name__}(server_url={self._client.base_url!r})"


class AsyncAlienManager(_operations.AsyncManager):
    """:class:`AlienManager` with ``async`` methods:
    ``await AsyncAlienManager(server_url, token).commands.create_command(...)``.
    """

    def __init__(self, server_url: str, token: str | None = None, *, timeout: float | None = DEFAULT_TIMEOUT) -> None:
        super().__init__(_manager_client(server_url, token, timeout))

    def __repr__(self) -> str:
        return f"{type(self).__name__}(server_url={self._client.base_url!r})"


class ProjectSetup:
    """Setup helpers that need the Platform API (``alien.setup``)."""

    def __init__(self, client: _gen.platform.PlatformClient) -> None:
        self._client = client

    def launch_url_for_project(
        self,
        project: str,
        region: str,
        stack_name: str,
        parameters: _typing.Mapping[str, str] | None = None,
    ) -> str:
        """Fetches the project's AWS template and builds a quick-create link for it."""
        return _call(_setup.launch_url_for_project_sync, self._client, project, region, stack_name, dict(parameters or {}))


class AsyncProjectSetup:
    """:class:`ProjectSetup` with ``async`` methods (``AsyncAlien(...).setup``)."""

    def __init__(self, client: _gen.platform.PlatformClient) -> None:
        self._client = client

    async def launch_url_for_project(
        self,
        project: str,
        region: str,
        stack_name: str,
        parameters: _typing.Mapping[str, str] | None = None,
    ) -> str:
        """Fetches the project's AWS template and builds a quick-create link for it."""
        return await _acall(_setup.launch_url_for_project, self._client, project, region, stack_name, dict(parameters or {}))


def cloudformation_launch_url(input: CloudFormationLaunch) -> str:
    """Builds an AWS CloudFormation quick-create link from a template URL."""
    return _call(_setup.cloudformation_launch_url_sync, input)


def shutdown() -> None:
    """Shuts the SDK's runtime down: blocks until calls in flight finish, then
    seals the telemetry recording. Calling it again does nothing. SDK calls
    made afterwards raise ``baml_bridge.BamlPanic``, which derives from
    ``BaseException`` (not ``Exception``) and says the runtime "has not been
    initialized".

    The runtime does this by itself at interpreter exit (``atexit``), which
    includes ``sys.exit()``. Unless telemetry is off (``ALIEN_TELEMETRY=off``),
    call ``shutdown()`` before the process ends without running ``atexit``
    handlers: ``os._exit()``, a serverless platform ending the process, or a
    signal such as SIGTERM, unless a handler turns it into ``sys.exit()``.
    Otherwise the recording is not sealed (docs/runtime.md).
    """
    _shutdown_runtime()


__all__ = [
    "Alien",
    "AlienError",
    "AlienManager",
    "AsyncAlien",
    "AsyncAlienManager",
    "AsyncProjectSetup",
    "CloudFormationLaunch",
    "DEFAULT_SERVER_URL",
    "DEFAULT_TIMEOUT",
    "ProjectSetup",
    "cloudformation_launch_url",
    "manager",
    "platform",
    "shutdown",
]
