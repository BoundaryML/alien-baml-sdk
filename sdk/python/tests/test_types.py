"""The request and response types, through their public import path:
``alien_sdk.platform`` and ``alien_sdk.manager`` (docs/api-shape.md,
"Validating input")."""

from __future__ import annotations

import importlib
import inspect
import sys

import pydantic
import pytest

import alien_sdk
from alien_sdk import _gen
from alien_sdk.platform import CreateSetupLinkRequest

FORM_WIRE = {"externalId": "cust-1", "name": "Customer", "project": "web", "maxDeployments": 5}
FORM_PYTHON = {"external_id": "cust-1", "name": "Customer", "project": "web", "max_deployments": 5}


@pytest.mark.parametrize("form", [FORM_WIRE, FORM_PYTHON], ids=["wire", "python"])
def test_a_form_validates_with_either_spelling(form: dict) -> None:
    request = CreateSetupLinkRequest.model_validate(form)
    assert request.external_id == "cust-1" and request.max_deployments == 5
    # The same request either way, sent with the wire names.
    assert request == CreateSetupLinkRequest.model_validate(FORM_WIRE)
    assert request.model_dump(by_alias=True, exclude_none=True) == FORM_WIRE
    assert CreateSetupLinkRequest.model_validate_json(request.model_dump_json(by_alias=True)) == request


def test_what_validation_accepts_and_rejects() -> None:
    # A missing required field fails, named by its wire name.
    with pytest.raises(pydantic.ValidationError) as caught:
        CreateSetupLinkRequest.model_validate({"name": "Customer", "project": "web"})
    assert [(e["loc"], e["type"]) for e in caught.value.errors()] == [(("externalId",), "missing")]
    # Values are converted as pydantic does by default (lax mode).
    assert CreateSetupLinkRequest.model_validate({**FORM_WIRE, "maxDeployments": "7"}).max_deployments == 7
    with pytest.raises(pydantic.ValidationError, match="maxDeployments"):
        CreateSetupLinkRequest.model_validate({**FORM_WIRE, "maxDeployments": "many"})
    # Unknown keys are ignored, a misspelled optional field included.
    request = CreateSetupLinkRequest.model_validate({**FORM_WIRE, "maxDeploymnets": 9, "extra": True})
    assert request.max_deployments == 5 and "extra" not in request.model_dump()
    # Both spellings of one field: the wire name wins.
    assert CreateSetupLinkRequest.model_validate({**FORM_WIRE, "external_id": "other"}).external_id == "cust-1"


@pytest.mark.parametrize("service", ["platform", "manager"])
def test_every_generated_type_is_exported(service: str) -> None:
    """``alien_sdk.<service>`` is a module of its own (``from alien_sdk.platform
    import X`` works) holding every class and type alias of the generated
    service module under its generated name, the very same object, and
    nothing else: no client, operation or group."""
    public = importlib.import_module(f"alien_sdk.{service}")
    assert getattr(alien_sdk, service) is public is sys.modules[f"alien_sdk.{service}"]
    generated = getattr(_gen, service)
    client = {"platform": "PlatformClient", "manager": "ManagerClient"}[service]
    expected = {
        name for name, value in vars(generated).items()
        if not name.startswith("_") and name != client and not inspect.ismodule(value) and not inspect.isfunction(value)
        and getattr(value, "__module__", None) == generated.__name__
    }
    assert expected <= set(public.__all__)
    assert set(public.__all__) == {name for name in vars(public) if not name.startswith("_") and name != "annotations"}
    assert client not in public.__all__
    for name in public.__all__:
        assert getattr(public, name) is getattr(generated, name), name
        assert not inspect.isfunction(getattr(public, name)) and not inspect.ismodule(getattr(public, name)), name


def test_the_two_apis_types_do_not_clash() -> None:
    """Names both APIs define stay apart, each in its own namespace."""
    shared = set(alien_sdk.platform.__all__) & set(alien_sdk.manager.__all__)
    assert "CreateCommandRequest" in shared
    assert alien_sdk.platform.CreateCommandRequest is not alien_sdk.manager.CreateCommandRequest
    assert not {"platform", "manager"} & (set(alien_sdk.platform.__all__) | set(alien_sdk.manager.__all__))
