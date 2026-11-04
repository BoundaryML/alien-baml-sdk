"""The generated SDK: the only module of the facade that names its package.

Every other module, and the methods ``genops.py`` generates, reach the
generated code through this one, so moving the generated package (for example
to ``alien_sdk._generated``) changes this import and nothing else. Importing
it starts the BAML runtime.
"""

from __future__ import annotations

from baml_sdk import _invocation_types, baml, errors, manager, platform, setup

__all__ = ["_invocation_types", "baml", "errors", "manager", "platform", "setup"]
