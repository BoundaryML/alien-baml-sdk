"""Starts conformance/stub-server for the session."""

from __future__ import annotations

import http.client
import json
import os
import subprocess
from pathlib import Path
from urllib.parse import urlsplit

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]
SECRETS: dict[str, str] = json.loads((REPO_ROOT / "conformance" / "secrets.json").read_text(encoding="utf-8"))


def pytest_configure(config: pytest.Config) -> None:
    """Refuses to run without a BAML_HOME other than ~/.baml: the runtime keeps
    state there, and tests must not touch the developer's. Runs before the
    test modules import the SDK, which starts the runtime."""
    home = os.environ.get("BAML_HOME", "")
    if not home or os.path.abspath(home) == os.path.join(Path.home(), ".baml"):
        raise pytest.UsageError(
            "BAML_HOME must name an isolated directory (not ~/.baml) before the tests load BAML.\n"
            "Run every suite with scripts/verify.sh, or this one by hand with a fresh home:\n"
            '  BAML_HOME="$(mktemp -d)" uv run --frozen --directory sdk/python pytest\n'
            '(conformance/README.md, "Running")'
        )


class Stub:
    def __init__(self) -> None:
        self.process = subprocess.Popen(
            ["node", str(REPO_ROOT / "conformance" / "stub-server" / "server.mjs")],
            stdout=subprocess.PIPE,
            text=True,
        )
        assert self.process.stdout is not None
        line = self.process.stdout.readline().strip()
        if not line.startswith("READY "):
            self.process.kill()
            raise RuntimeError(f"stub server did not start: {line!r}")
        self.base = line[len("READY "):]
        url = urlsplit(self.base)
        # One kept-alive connection for every control request: a connection
        # per request leaves a TIME_WAIT socket each, and runs in parallel on
        # one machine then fail to connect. http.client never goes through a
        # proxy, which a stub on 127.0.0.1 must not.
        self.connection = http.client.HTTPConnection(url.hostname, url.port, timeout=60)

    def get(self, path: str) -> dict:
        return self._request("GET", path, None)

    def post(self, path: str, body: object) -> dict:
        return self._request("POST", path, json.dumps(body).encode())

    def _request(self, method: str, path: str, payload: bytes | None) -> dict:
        headers = {} if payload is None else {"content-type": "application/json"}
        for attempt in (1, 2):
            try:
                self.connection.request(method, path, body=payload, headers=headers)
                response = self.connection.getresponse()
                data = response.read()
                break
            except (http.client.HTTPException, ConnectionError):
                # The stub closes a connection idle for 5 s: reconnect once.
                self.connection.close()
                if attempt == 2:
                    raise
        if response.status != 200:
            raise RuntimeError(f"stub {path} failed: HTTP {response.status}: {data[:1000]!r}")
        return json.loads(data)

    def stop(self) -> None:
        self.connection.close()
        self.process.terminate()
        self.process.wait(timeout=10)


@pytest.fixture(scope="session")
def secrets() -> dict[str, str]:
    """conformance/secrets.json: fixture credentials that telemetry check T3 looks for in recordings."""
    return SECRETS


@pytest.fixture(scope="session")
def stub():
    server = Stub()
    yield server
    server.stop()
