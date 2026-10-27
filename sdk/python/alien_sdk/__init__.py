"""Alien SDK for Python.

A thin facade over the generated BAML SDK (``baml_sdk``): it builds clients,
renames, converts errors and picks the public surface. Every behavior lives in
the BAML project (``baml/baml_src``).

``platform`` and ``manager`` are the generated modules, exported for their
request and response types (``platform.CreateSetupLinkRequest``), which are
also what ``isinstance`` checks need: a union member this SDK does not know
is ``platform.EventDataOther`` and the like (docs/api-shape.md). Call
operations through :class:`Alien` and :class:`AlienManager` (or
:class:`AsyncAlien` and :class:`AsyncAlienManager`): the functions in those
modules take the client as their first argument and raise
``baml_bridge.BamlError`` instead of :class:`AlienError`.
"""

from __future__ import annotations

import os as _os
import typing as _typing

import pydantic as _pydantic
from baml_bridge import shutdown_runtime as _shutdown_runtime

# Importing the generated SDK starts the runtime.
from baml_sdk import manager, platform  # (generated types: request and response shapes)
from baml_sdk import setup as _setup

from ._bind import Bound as _Bound
from ._errors import AlienError, _converted

CloudFormationLaunch = _setup.CloudFormationLaunch

DEFAULT_SERVER_URL = "https://api.alien.dev"


# The generated client classes are pydantic models, whose repr() shows every
# field. These subclasses leave the credential out of repr(), vars() and
# debuggers. A traceback that captures locals (as error reporters do) still
# holds it: baml_bridge keeps the encoded call in a local of its own frame.
class _PlatformClient(platform.PlatformClient):
    api_key: str = _pydantic.Field(repr=False)


class _ManagerClient(manager.ManagerClient):
    token: _typing.Optional[str] = _pydantic.Field(default=None, repr=False)


def _platform_client(api_key: str | None, server_url: str | None, workspace: str | None, timeout_ms: int | None) -> _PlatformClient:
    if api_key is None:
        api_key = _os.environ.get("ALIEN_API_KEY")
    if not api_key:
        raise AlienError("INVALID_REQUEST", "no API key: pass api_key or set ALIEN_API_KEY", False)
    if server_url is None:
        server_url = DEFAULT_SERVER_URL
    return _PlatformClient(base_url=server_url, api_key=api_key, workspace=workspace, timeout_ms=timeout_ms)


def _manager_client(server_url: str, token: str | None, timeout_ms: int | None) -> _ManagerClient:
    # An empty token sends no Authorization header.
    return _ManagerClient(base_url=server_url, token=token or None, timeout_ms=timeout_ms)


class Alien(_Bound):
    """A Platform API client, configured like Alien's TypeScript SDK
    (``@alienplatform/platform-api``): ``api_key`` defaults to the
    ``ALIEN_API_KEY`` environment variable, ``server_url`` (when ``None``) to
    :data:`DEFAULT_SERVER_URL`, and ``workspace`` is optional for
    workspace-scoped API keys.

    ``timeout_ms`` limits each HTTP request, from connecting to reading the
    whole response (Alien's ``timeoutMs``). ``None``: BAML's default of 5
    minutes; 0 or negative: no limit. A request that runs out raises
    :class:`AlienError` ``COMMUNICATION_ERROR`` (retryable). A per-call
    ``_baml`` timeout limits the whole call as well: the shorter one ends it.

    ``alien.<group>.<method>(...)`` calls any Platform API operation::

        alien = Alien(workspace="my-workspace")
        m = alien.managers.get("mgr_...")
        page = alien.deployments.list(project="web", deploymentGroup="prod", limit=10)
        page.nextCursor

    Groups and methods are snake_case. Required arguments are positional and
    optional ones are keywords. Keywords and fields keep the API's camelCase
    names (``deploymentGroup=``, ``page.nextCursor``). Any other keyword,
    such as ``deployment_group=``, raises ``baml_bridge.BamlError``
    (``InvalidArgument``), not ``TypeError``.

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
        timeout_ms: int | None = None,
    ) -> None:
        client = _platform_client(api_key, server_url, workspace, timeout_ms)
        super().__init__(platform, client)
        self.setup = ProjectSetup(client)

    def __repr__(self) -> str:
        return f"{type(self).__name__}(server_url={self._client.base_url!r}, workspace={self._client.workspace!r})"


class AsyncAlien(_Bound):
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
        timeout_ms: int | None = None,
    ) -> None:
        client = _platform_client(api_key, server_url, workspace, timeout_ms)
        super().__init__(platform, client, asynchronous=True)
        self.setup = AsyncProjectSetup(client)

    __repr__ = Alien.__repr__


class AlienManager(_Bound):
    """A Manager API client, for example for the ``managerUrl`` and ``token`` from
    ``alien.commands.bootstrap``. ``AlienManager(server_url, token).commands.create_command(...)``
    calls any Manager API operation; the token is sent on every operation
    when set (an empty token sends none). ``timeout_ms`` is :class:`Alien`'s.
    Call :func:`shutdown` before the process ends without running ``atexit``
    handlers (a signal such as SIGTERM, ``os._exit()``).
    """

    def __init__(self, server_url: str, token: str | None = None, *, timeout_ms: int | None = None) -> None:
        super().__init__(manager, _manager_client(server_url, token, timeout_ms))

    def __repr__(self) -> str:
        return f"{type(self).__name__}(server_url={self._client.base_url!r})"


class AsyncAlienManager(_Bound):
    """:class:`AlienManager` with ``async`` methods:
    ``await AsyncAlienManager(server_url, token).commands.create_command(...)``.
    """

    def __init__(self, server_url: str, token: str | None = None, *, timeout_ms: int | None = None) -> None:
        super().__init__(manager, _manager_client(server_url, token, timeout_ms), asynchronous=True)

    __repr__ = AlienManager.__repr__


class ProjectSetup:
    """Setup helpers that need the Platform API (``alien.setup``)."""

    def __init__(self, client: platform.PlatformClient) -> None:
        self._client = client

    def launch_url_for_project(
        self,
        project: str,
        region: str,
        stack_name: str,
        parameters: _typing.Mapping[str, str] | None = None,
    ) -> str:
        """Fetches the project's AWS template and builds a quick-create link for it."""
        with _converted():
            return _setup.launch_url_for_project(self._client, project, region, stack_name, dict(parameters or {}))


class AsyncProjectSetup:
    """:class:`ProjectSetup` with ``async`` methods (``AsyncAlien(...).setup``)."""

    def __init__(self, client: platform.PlatformClient) -> None:
        self._client = client

    async def launch_url_for_project(
        self,
        project: str,
        region: str,
        stack_name: str,
        parameters: _typing.Mapping[str, str] | None = None,
    ) -> str:
        """Fetches the project's AWS template and builds a quick-create link for it."""
        with _converted():
            return await _setup.launch_url_for_project_async(self._client, project, region, stack_name, dict(parameters or {}))


def cloudformation_launch_url(input: CloudFormationLaunch) -> str:
    """Builds an AWS CloudFormation quick-create link from a template URL."""
    with _converted():
        return _setup.cloudformation_launch_url(input)


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
    "ProjectSetup",
    "cloudformation_launch_url",
    "manager",
    "platform",
    "shutdown",
]
