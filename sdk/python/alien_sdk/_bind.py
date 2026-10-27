"""Binds a generated BAML namespace to a client, without per-operation code.

``Alien(...).deployment_groups.create_token(...)`` calls
``baml_sdk.platform.deploymentGroups.createToken(client, ...)``: attribute names
are snake_case, group submodules are bound the same way, and BAML-thrown
AlienErrors become ``alien_sdk.AlienError``.
"""

from __future__ import annotations

import functools
import re
import types
from typing import Any

from ._errors import _converted


def _snake(name: str) -> str:
    """``createToken`` → ``create_token``, ``createAPIKey`` → ``create_api_key``,
    ``getGcpOAuthProvider`` → ``get_gcp_oauth_provider`` (OAuth is one word)."""
    name = name.replace("OAuth", "Oauth")
    return re.sub(r"(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])", "_", name).lower()


class Bound:
    def __init__(self, module: types.ModuleType, client: Any, *, asynchronous: bool = False) -> None:
        self._module = module
        self._client = client
        # Asynchronous: bind the generated ``<name>_async`` functions instead.
        self._asynchronous = asynchronous

    @functools.cached_property
    def _names(self) -> dict[str, str]:
        """snake_case name → generated name, for operations and groups only."""
        groups = set(getattr(self._module, "_LAZY_CHILDREN", ()))
        operations = {
            name
            for name in getattr(self._module, "__all__", ())
            if not name.startswith("_")
            and not name.endswith("_async")
            and isinstance(self._module.__dict__.get(name), types.FunctionType)
        }
        return {_snake(name): name for name in groups | operations}

    def __getattr__(self, name: str) -> Any:
        # Private names never resolve to operations (copy and pickle probe them
        # before __init__ has run).
        if name.startswith("_"):
            raise AttributeError(name)
        target = self._names.get(name)
        if target is None:
            raise AttributeError(f"{self._module.__name__} has no operation or group {name!r}")
        value = getattr(self._module, target)
        if isinstance(value, types.ModuleType):
            result: Any = Bound(value, self._client, asynchronous=self._asynchronous)
        elif self._asynchronous:
            function = getattr(self._module, f"{target}_async")

            @functools.wraps(function)
            async def result(*args: Any, **kwargs: Any) -> Any:
                with _converted():
                    return await function(self._client, *args, **kwargs)

        else:

            @functools.wraps(value)
            def result(*args: Any, **kwargs: Any) -> Any:
                with _converted():
                    return value(self._client, *args, **kwargs)

        self.__dict__[name] = result  # later lookups skip __getattr__
        return result

    def __dir__(self) -> list[str]:
        # Operations and groups, plus the facade's own attributes (``alien.setup``).
        return sorted({*self._names, *(name for name in self.__dict__ if not name.startswith("_"))})
