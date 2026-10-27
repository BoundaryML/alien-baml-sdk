from __future__ import annotations

import contextlib
from collections.abc import Iterator, Mapping

from baml_bridge import BamlError
from baml_sdk import errors as _errors


class AlienError(Exception):
    """The error every Alien SDK call raises (docs/errors.md).

    ``str(error)`` is ``"<code>: <message>"``; the fields are ``code``,
    ``message``, ``retryable``, ``http_status`` and ``context``.
    """

    # Tracebacks and pickle name the public class, alien_sdk.AlienError.
    __module__ = "alien_sdk"

    def __init__(
        self,
        code: str,
        message: str,
        retryable: bool,
        http_status: int | None = None,
        context: Mapping[str, str] | None = None,
    ) -> None:
        # All constructor arguments go to Exception.args so pickle and copy work.
        super().__init__(code, message, retryable, http_status, context)
        self.code = code
        self.message = message
        self.retryable = retryable
        self.http_status = http_status
        self.context = dict(context) if context is not None else None

    def __str__(self) -> str:
        return f"{self.code}: {self.message}"

    def __repr__(self) -> str:
        return f"AlienError(code={self.code!r}, retryable={self.retryable!r}, http_status={self.http_status!r}, message={self.message!r})"


@contextlib.contextmanager
def _converted() -> Iterator[None]:
    """Re-raises a BAML-thrown AlienError as ``alien_sdk.AlienError``; anything else propagates unchanged."""
    try:
        yield
    except BamlError as error:
        value = error.value
        if isinstance(value, _errors.AlienError):
            raise AlienError(value.code, value.message, value.retryable, value.http_status, value.context) from error
        raise
