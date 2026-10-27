"""B4 repro: two BAML-generated SDKs installed into one Python environment.

Prints one JSON line per call so run.sh can assert the observed behavior.
"""

import json
import sys


def attempt(name, fn):
    try:
        print(json.dumps({"call": name, "ok": fn()}, separators=(",", ":")))
    except BaseException as error:  # BamlPanic derives from BaseException
        first_line = (str(error).splitlines() or [""])[0]
        print(json.dumps({"call": name, "error": f"{type(error).__name__}: {first_line}"}, separators=(",", ":")))


def alien_call():
    import alien_sdk

    return alien_sdk.cloudformation_launch_url(alien_sdk.CloudFormationLaunch(
        templateUrl="https://example.com/setup.yaml", region="us-east-1", stackName="alien-setup", parameters={}))


def other_call():
    import baml_sdk

    return baml_sdk.greet("Alien")


def vendored_call():
    import other_baml_sdk

    return other_baml_sdk.greet("Alien")


# app.py [vendored]: the other SDK is importable as `other_baml_sdk`.
if sys.argv[1:] == ["vendored"]:
    attempt("vendored.import_order", lambda: "alien_sdk, then other_baml_sdk")
    import alien_sdk  # noqa: F401  (starts the runtime with the Alien program)
    attempt("vendored.greet", vendored_call)  # importing it starts a second runtime
    attempt("alien.cloudformation_launch_url", alien_call)
else:
    attempt("alien.cloudformation_launch_url", alien_call)
    attempt("other.greet", other_call)
