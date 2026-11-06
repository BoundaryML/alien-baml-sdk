"""The generated SDK: the only module of the facade that names its package.

BAML generates it inside the facade, as ``alien_sdk._generated``
(``package_name`` in ``baml/baml.toml``), so installing the SDK writes no
top-level package that another BAML-generated SDK could overwrite. Every other
module, and the methods ``genops.py`` generates, reach the generated code
through this one, so renaming the generated package changes this import and
nothing else. Importing it starts the BAML runtime.
"""

from __future__ import annotations

from ._generated import _invocation_types, baml, errors, manager, platform, setup

__all__ = ["_invocation_types", "baml", "errors", "manager", "platform", "setup"]
