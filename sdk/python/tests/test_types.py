"""The request and response types, through their public import path:
``alien_sdk.platform`` and ``alien_sdk.manager`` (docs/api-shape.md,
"Validating input")."""

from __future__ import annotations

import importlib
import inspect
import json
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


def test_json_attributes_are_not_followed_by_pydantic() -> None:
    """B27 (docs/gaps.md): the pydantic models do not follow the ``@json``
    attributes, so for a renamed field, a class's unlisted keys and a union's
    ``Other`` member, ``model_validate`` does not read the API's JSON and
    ``model_dump(by_alias=True)`` does not write it. The SDK's own encoding,
    ``baml.json``, does."""
    rollout = {"updated": 1, "updating": 0, "failed": 0, "pending": 0, "superseded": 0,
               "waiting-on-owner": 0, "waiting-on-inputs": 0, "pinned-other": 0, "on-other": 2}
    # @json.rename: the alias is the BAML name (onOther), not the wire name (on-other).
    with pytest.raises(pydantic.ValidationError) as caught:
        alien_sdk.platform.ReleaseRolloutStateCounts.model_validate(rollout)
    assert [e["loc"] for e in caught.value.errors()] == [("waitingOnOwner",), ("waitingOnInputs",), ("pinnedOther",), ("onOther",)]
    counts = _gen.baml.json.to_sync(rollout, _types={"T": alien_sdk.platform.ReleaseRolloutStateCounts})
    assert counts.on_other == 2
    assert counts.model_dump(by_alias=True)["onOther"] == 2 and "on-other" not in counts.model_dump(by_alias=True)
    assert json.loads(_gen.baml.json.stringify_sync(counts)) == rollout
    # A reserved word (wire `test`, field `test_`) has no alias: the wire key is dropped.
    assert alien_sdk.platform.StackByPlatform.model_validate({"test": {"id": "t"}}).test_ is None
    stack = alien_sdk.platform.StackByPlatform(test_={"id": "t"})
    assert stack.model_dump(by_alias=True, exclude_none=True) == {"test_": {"id": "t"}}
    assert json.loads(_gen.baml.json.stringify_sync(stack)) == {"test": {"id": "t"}}
    # A wire name that is also the Python name validates, but dumps as the alias.
    network = alien_sdk.manager.NetworkSettingsByoVpcAws.model_validate(
        {"type": "byo-vpc-aws", "vpc_id": "vpc-1", "public_subnet_ids": ["a"], "private_subnet_ids": ["b"]})
    assert network.vpc_id == "vpc-1" and "vpcId" in network.model_dump(by_alias=True)
    # @json.flatten: the unlisted keys are wanted under `additionalProperties`.
    config = {"id": "r1", "type": "function", "memoryMb": 512}
    with pytest.raises(pydantic.ValidationError, match="additionalProperties"):
        alien_sdk.platform.DeploymentStackStateResourcesValueConfig.model_validate(config)
    decoded = _gen.baml.json.to_sync(config, _types={"T": alien_sdk.platform.DeploymentStackStateResourcesValueConfig})
    assert decoded.additional_properties == {"memoryMb": 512}
    # @json.fallback: a member this SDK does not know fits no member in pydantic.
    unknown = {"mode": "future-mode", "x": 1}
    with pytest.raises(pydantic.ValidationError):
        pydantic.TypeAdapter(alien_sdk.platform.ComputePoolSelection).validate_python(unknown)
    other = _gen.baml.json.to_sync(unknown, _types={"T": alien_sdk.platform.ComputePoolSelection})
    assert other == alien_sdk.platform.ComputePoolSelectionOther(mode="UNKNOWN", raw=unknown)
