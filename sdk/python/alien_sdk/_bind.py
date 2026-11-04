"""Binds a generated BAML namespace to a client, without per-operation code.

``Alien(...).deployment_groups.create_token(...)`` calls
``baml_sdk.platform.deployment_groups.create_token_sync(client, ...)``, and
``AsyncAlien(...)`` the async ``create_token(client, ...)``: attributes are the
generated names (snake_case, BAML's ``language`` naming), group
submodules are bound the same way, and BAML-thrown AlienErrors become
``alien_sdk.AlienError``.
"""

from __future__ import annotations

import functools
import types
from typing import Any

from ._errors import _converted

# What the generated SDK appends to an operation's (async) name for its blocking twin.
_SYNC_SUFFIX = "_sync"


def _members(module: types.ModuleType) -> frozenset[str]:
    """The generated module's groups (submodules) and operations, by name: an
    operation is an async function with a blocking ``<name>_sync`` twin."""
    groups = set(getattr(module, "_LAZY_CHILDREN", ()))
    names = set(getattr(module, "__all__", ()))
    operations = {
        name
        for name in names
        if not name.startswith("_")
        and name + _SYNC_SUFFIX in names
        and isinstance(module.__dict__.get(name), types.FunctionType)
        and isinstance(module.__dict__.get(name + _SYNC_SUFFIX), types.FunctionType)
    }
    return frozenset(groups | operations)


class Bound:
    def __init__(self, module: types.ModuleType, client: Any, *, asynchronous: bool = False) -> None:
        self._module = module
        self._client = client
        # Asynchronous: bind the generated async ``<name>`` functions;
        # otherwise their blocking ``<name>_sync`` twins.
        self._asynchronous = asynchronous

    @functools.cached_property
    def _names(self) -> frozenset[str]:
        return _members(self._module)

    def __getattr__(self, name: str) -> Any:
        # Private names never resolve to operations (copy and pickle probe them
        # before __init__ has run).
        if name.startswith("_"):
            raise AttributeError(name)
        if name not in self._names:
            raise AttributeError(f"{self._module.__name__} has no operation or group {name!r}")
        value = getattr(self._module, name)
        if isinstance(value, types.ModuleType):
            result: Any = Bound(value, self._client, asynchronous=self._asynchronous)
        elif self._asynchronous:

            @functools.wraps(value)
            async def result(*args: Any, **kwargs: Any) -> Any:
                with _converted():
                    return await value(self._client, *args, **kwargs)

        else:
            function = getattr(self._module, name + _SYNC_SUFFIX)

            @functools.wraps(function)
            def result(*args: Any, **kwargs: Any) -> Any:
                with _converted():
                    return function(self._client, *args, **kwargs)

        self.__dict__[name] = result  # later lookups skip __getattr__
        return result

    def __dir__(self) -> list[str]:
        # Operations and groups, plus the facade's own attributes (``alien.setup``).
        return sorted({*self._names, *(name for name in self.__dict__ if not name.startswith("_"))})
