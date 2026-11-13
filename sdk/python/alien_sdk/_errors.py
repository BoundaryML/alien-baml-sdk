from __future__ import annotations

from collections.abc import Awaitable, Callable
from typing import Any, ParamSpec, TypeVar

from baml_bridge import BamlError
from ._gen import errors as _errors

_P = ParamSpec("_P")
_T = TypeVar("_T")


class AlienError(Exception):
    """The error every Alien SDK call raises (docs/errors.md).

    ``str(error)`` is ``"<code>: <message>"``; the fields are ``code``,
    ``message``, ``retryable``, ``http_status``, ``context``, ``hint``,
    ``request_id``, ``source`` and ``internal``.

    ``context`` is JSON data, typed ``Any`` as the generated SDK types
    ``json``: for a server error, the error body's ``context`` exactly as sent
    (a ``dict``, ``list``, ``str``, ``int``, ``float``, ``bool`` or ``None``);
    for an error the SDK raises itself, ``None`` or a ``dict`` with the SDK's
    own keys (docs/errors.md). It is stored as given, not copied.

    ``request_id`` is the server's request ID: the error body's
    ``requestId``, else the response's ``x-request-id`` header; ``None`` when
    no response arrived or it named none.

    ``source`` is the error the server names as this one's cause (its own
    ``source`` in turn, at most 8 levels below the raised error), and
    ``__cause__`` is ``source``, so tracebacks and anything that walks
    ``__cause__`` show the server's chain. The BAML runtime's own error, which
    carried the failure, is neither ``__cause__`` nor ``__context__``.
    """

    # Tracebacks and pickle name the public class, alien_sdk.AlienError.
    __module__ = "alien_sdk"

    def __init__(
        self,
        code: str,
        message: str,
        retryable: bool,
        http_status: int | None = None,
        context: Any = None,
        hint: str | None = None,
        request_id: str | None = None,
        source: AlienError | None = None,
        internal: bool | None = None,
    ) -> None:
        # All constructor arguments go to Exception.args so pickle and copy work.
        super().__init__(code, message, retryable, http_status, context, hint, request_id, source, internal)
        self.code = code
        self.message = message
        self.retryable = retryable
        self.http_status = http_status
        self.context = context
        self.hint = hint
        self.request_id = request_id
        self.source = source
        self.internal = internal
        if source is not None:
            # Here rather than with `raise ... from`, so an error that is built
            # directly, unpickled or copied is chained too. Setting it also
            # sets __suppress_context__.
            self.__cause__ = source

    def __str__(self) -> str:
        return f"{self.code}: {self.message}"

    def __repr__(self) -> str:
        return f"AlienError(code={self.code!r}, retryable={self.retryable!r}, http_status={self.http_status!r}, message={self.message!r})"


def _from_generated(value: _errors.AlienError) -> AlienError:
    """The generated error class as ``alien_sdk.AlienError``, field by field,
    ``source`` level by level."""
    return AlienError(
        value.code,
        value.message,
        value.retryable,
        value.http_status,
        value.context,
        value.hint,
        value.request_id,
        _from_generated(value.source) if value.source is not None else None,
        value.internal,
    )


def _thrown(error: BamlError) -> AlienError | None:
    """The ``alien_sdk.AlienError`` for a BAML-thrown AlienError; ``None`` for anything else."""
    value = error.value
    return _from_generated(value) if isinstance(value, _errors.AlienError) else None


# Both wrappers raise the converted error after their `except` block has
# ended, never inside it and never from a context manager's __exit__: an
# exception raised while another is being handled gets that one as its
# __context__. The BamlError's traceback holds baml_bridge's frame, whose
# locals hold the call's arguments, the client with its credential among them
# (docs/gaps.md#b23), so the AlienError must not lead to the BamlError at
# all. Once the block ends, nothing refers to the BamlError.


def _call(fn: Callable[_P, _T], /, *args: _P.args, **kwargs: _P.kwargs) -> _T:
    """``fn(*args, **kwargs)``; a BAML-thrown AlienError is raised as
    ``alien_sdk.AlienError``, anything else propagates unchanged."""
    try:
        return fn(*args, **kwargs)
    except BamlError as error:
        converted = _thrown(error)
        if converted is None:
            raise
    raise converted


async def _acall(fn: Callable[_P, Awaitable[_T]], /, *args: _P.args, **kwargs: _P.kwargs) -> _T:
    """:func:`_call` for an ``async`` function: awaits ``fn(*args, **kwargs)``."""
    try:
        return await fn(*args, **kwargs)
    except BamlError as error:
        converted = _thrown(error)
        if converted is None:
            raise
    raise converted
