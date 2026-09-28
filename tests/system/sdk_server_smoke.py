"""Pinned E2B SDK against the real Sandproof management and envd gateway.

Requires an isolated Linux/KVM server. Never points at the public E2B cloud.
Secrets come from the caller's environment and are never printed.
"""

import argparse
import importlib.metadata
import json
import os
import time

import httpx
from e2b import Sandbox
from e2b.exceptions import SandboxNotFoundException


def options(key: str, api_url: str, sandbox_url: str) -> dict:
    return dict(
        api_url=api_url,
        sandbox_url=sandbox_url,
        api_key=key,
        domain="example.invalid",
        retries=0,
        request_timeout=30,
    )


def exercise(args: argparse.Namespace) -> None:
    assert importlib.metadata.version("e2b") == "2.51.0"
    owner_key = os.environ["SANDPROOF_TEST_API_KEY"]
    foreign_key = os.environ.get("SANDPROOF_TEST_FOREIGN_API_KEY")
    owner = options(owner_key, args.api_url, args.sandbox_url)
    foreign = options(foreign_key, args.api_url, args.sandbox_url) if foreign_key else None
    sandbox = None
    try:
        sandbox = Sandbox.create(
            template="base", timeout=180, allow_internet_access=False, **owner
        )
        ident = sandbox.sandbox_id
        assert Sandbox.connect(ident, **owner).sandbox_id == ident
        assert sandbox.get_info().state == "running"
        assert [item.sandbox_id for item in Sandbox.list(**owner).next_items()] == [ident]
        Sandbox.set_timeout(ident, 180, **owner)

        if foreign:
            try:
                Sandbox.connect(ident, **foreign)
            except SandboxNotFoundException:
                pass
            else:
                raise AssertionError("foreign tenant connected to owner's sandbox")
            assert Sandbox.kill(ident, **foreign) is False
            assert Sandbox.list(**foreign).next_items() == []

        assert sandbox.commands.run("printf controller-ok").stdout == "controller-ok"
        try:
            sandbox.commands.run("sleep 3", timeout=1)
        except Exception as error:
            assert type(error).__name__ == "TimeoutException", type(error).__name__
        else:
            raise AssertionError("command timeout was ignored")

        handle = sandbox.commands.run("sleep 2; printf reconnect-ok", background=True)
        process_id = handle.pid
        handle.disconnect()
        assert sandbox.commands.connect(process_id).wait().stdout == "reconnect-ok"
        handle = sandbox.commands.run("sleep 30", background=True)
        assert handle.kill() is True
        large = sandbox.commands.run("head -c 1048576 /dev/zero | tr '\\000' Z").stdout
        assert len(large) == 1_048_576 and large.startswith("Z")

        sandbox.files.write("/tmp/sandproof-gateway.txt", "file-ok")
        assert sandbox.files.read("/tmp/sandproof-gateway.txt") == "file-ok"
        assert sandbox.files.make_dir("/tmp/sandproof-gateway-dir")
        assert sandbox.files.list("/tmp", depth=1)
        guest_hostname = sandbox.commands.run("cat /etc/hostname").stdout
        assert sandbox.files.read("/proc/1/root/etc/hostname") == guest_hostname

        token = sandbox._envd_access_token
        with httpx.Client(base_url=args.sandbox_url, trust_env=False, timeout=10) as client:
            assert client.get("/files", params={"path": "/tmp/x"}, headers={"X-Access-Token": "0" * 64}).status_code == 401
            assert client.get("/files?path=%00bad", headers={"X-Access-Token": token}).status_code == 400
            assert client.get("/files?path=%FF", headers={"X-Access-Token": token}).status_code == 400
            assert client.get(
                "/files", headers=[("X-Access-Token", token), ("X-Access-Token", token)]
            ).status_code == 401
            assert client.post(
                "/files", content=b"x" * (8 * 1024 * 1024 + 1),
                headers={"X-Access-Token": token},
            ).status_code == 413
        assert sandbox.commands.run("printf after-rejection").stdout == "after-rejection"
    finally:
        if sandbox is not None:
            assert Sandbox.kill(sandbox.sandbox_id, **owner)
            with httpx.Client(base_url=args.sandbox_url, trust_env=False, timeout=10) as client:
                assert client.get("/health", headers={"X-Access-Token": sandbox._envd_access_token}).status_code == 401

    expiring = Sandbox.create(
        template="base", timeout=5, allow_internet_access=False, **owner
    )
    for _ in range(12):
        time.sleep(1)
        if expiring.sandbox_id not in [item.sandbox_id for item in Sandbox.list(**owner).next_items()]:
            break
    else:
        Sandbox.kill(expiring.sandbox_id, **owner)
        raise AssertionError("deadline scanner did not destroy sandbox")
    print(json.dumps({
        "pinned_sdk": "2.51.0",
        "management": "create/connect/get/list/set_timeout/kill",
        "data": "command/timeout/kill/reconnect/large-output/files/directories",
        "authorization": "foreign-tenant/wrong-token/revoked-token/NUL-or-invalid-UTF8-path/duplicate-token/oversized-upload",
        "expiry": "automatic kill observed",
        "scope": "isolated real VM; template API, egress, Harbor and proofs not covered",
    }))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-url", required=True)
    parser.add_argument("--sandbox-url", required=True)
    exercise(parser.parse_args())
