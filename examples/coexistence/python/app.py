"""B4 repro: two BAML-generated SDKs installed into one Python environment.

    python app.py installed                  each SDK's installed files are intact
    python app.py alien-first | other-first  imports both SDKs in that order, then calls both

Prints one JSON line per call so run.sh can assert the observed behavior.
"""

import base64
import hashlib
import importlib
import importlib.metadata
import json
import sys


def attempt(name, fn):
    try:
        print(json.dumps({"call": name, "ok": fn()}, separators=(",", ":")))
    except BaseException as error:  # BamlPanic derives from BaseException
        first_line = (str(error).splitlines() or [""])[0]
        print(json.dumps({"call": name, "error": f"{type(error).__name__}: {first_line}"}, separators=(",", ":")))


def intact(distribution):
    """Every file the distribution installed is there, with the hash its
    RECORD lists: installing or removing the other SDK overwrote none."""
    files = importlib.metadata.files(distribution) or []
    hashed = [f for f in files if f.hash]
    for f in hashed:
        digest = hashlib.new(f.hash.mode, f.locate().read_bytes()).digest()
        if base64.urlsafe_b64encode(digest).rstrip(b"=").decode() != f.hash.value:
            raise AssertionError(f"{f} is not the file {distribution} installed")
    top = sorted({f.parts[0] for f in files if not f.parts[0].endswith(".dist-info")})
    return f"{len(hashed)} files intact, top-level [{' '.join(top)}]"


def alien_call():
    import alien_sdk

    return alien_sdk.cloudformation_launch_url(alien_sdk.CloudFormationLaunch(
        template_url="https://example.com/setup.yaml", region="us-east-1", stack_name="alien-setup", parameters={}))


def other_call():
    import baml_sdk

    return baml_sdk.greet_sync("Alien")


ORDERS = {"alien-first": ["alien_sdk", "baml_sdk"], "other-first": ["baml_sdk", "alien_sdk"]}
mode = sys.argv[1] if len(sys.argv) == 2 else ""
if mode == "installed":
    attempt("installed.alien-sdk", lambda: intact("alien-sdk"))
    attempt("installed.other-baml-sdk", lambda: intact("other-baml-sdk"))
elif mode in ORDERS:
    for module in ORDERS[mode]:  # importing an SDK starts its runtime
        attempt(f"import.{module}", lambda: importlib.import_module(module).__name__)
    attempt("alien.cloudformation_launch_url", alien_call)
    attempt("other.greet", other_call)
else:
    sys.exit("usage: python app.py installed | alien-first | other-first")
