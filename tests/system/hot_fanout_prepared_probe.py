#!/usr/bin/env python3
"""Exercise a prepared candidate's vsock and stock SDK inside its private VM network."""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import importlib.metadata
import json
from pathlib import Path
import re
import secrets
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools.hot_fanout.service import MAX_MESSAGE, call_firecracker_vsock
from hot_fanout_guest import probe_sdk, require
from hot_fanout_host import SDK_COMMIT

FIXED_ADDRESS = "172.16.0.2"


def sdk_probe():
    data = sys.stdin.buffer.read(MAX_MESSAGE + 1)
    require(len(data) <= MAX_MESSAGE, "invalid SDK test plan bound")
    request = json.loads(data)
    package = importlib.metadata.distribution("e2b")
    provenance = json.loads(package.read_text("direct_url.json"))
    require(package.version == "2.51.0" and provenance["vcs_info"]["commit_id"] == SDK_COMMIT,
            "expected pinned SDK installed in this interpreter")
    probe_sdk(request["branches"], request["expected_memory_mark"], FIXED_ADDRESS)
    return {"sdk_branches": len(request["branches"])}


def exercise(args):
    socket_path = args.vsock.resolve()
    require(socket_path.is_socket() and socket_path.name == "fanout.vsock",
            "select the exact prepared fixture's Firecracker host socket")
    require(re.fullmatch(r"vm-[0-9a-f]{16}-[1-9][0-9]*", args.vm_netns) is not None,
            "select an existing Sandproof fixture VM network namespace")
    with args.receipt.open("rb") as stream:
        data = stream.read(64 * 1024 + 1)
    require(len(data) <= 64 * 1024, "invalid fixture receipt size")
    receipt = json.loads(data)
    pin = receipt["native_hot_fanout"]["template"]
    require(type(pin["max_branches"]) is int and 1 <= args.n <= pin["max_branches"] <= 64,
            "N must fit the approved guest checkpoint")

    def call(request):
        return call_firecracker_vsock(socket_path, request, 15)

    require(call({"operation": "probe"}) ==
            {"status": "checkpoint_ready", "checkpoint_id": pin["checkpoint_id"],
             "max_branches": pin["max_branches"]},
            "guest does not expose the approved prepared checkpoint")
    batch = "prepared-" + secrets.token_hex(12)
    branches = [
        {"branch_id": f"{batch}_{i}", "port": 31000 + i, "access_token": secrets.token_urlsafe(32)}
        for i in range(args.n)
    ]
    request = {"operation": "start", "plan": {"batch_id": batch, "checkpoint_id": pin["checkpoint_id"],
                                               "branches": branches}}
    began = time.monotonic()
    try:
        result = call(request)
        expected = [{field: item[field] for field in ("branch_id", "port")} for item in branches]
        require(result == {"batch_id": batch, "status": "ready", "branches": expected, "released": []},
                "prepared candidate did not produce the original complete batch")
        ready = time.monotonic()
        plan = json.dumps({"branches": branches, "expected_memory_mark": receipt["expected_inherited_mark"]})
        check = subprocess.run(
            ["/usr/bin/ip", "netns", "exec", args.vm_netns, sys.executable, str(Path(__file__).resolve()),
             "--sdk-child"],
            input=plan, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, timeout=50, check=False,
        )
        require(check.returncode == 0 and json.loads(check.stdout) == {"sdk_branches": args.n},
                "pinned SDK command/file or inherited-memory checks failed")
        return {"branches": args.n, "checkpoint": pin["checkpoint_id"],
                "vsock_ready_ms": (ready - began) * 1000,
                "sdk_ready_ms": (time.monotonic() - began) * 1000}
    finally:
        settled = {"batch_id": batch, "status": "released"}
        # A startup failure may have already completed exact partial cleanup.
        release = call({"operation": "release", "batch_id": batch})
        require(release in (settled, {"batch_id": batch, "status": "failed_clean"}),
                "prepared test batch cleanup remains unresolved")
        require(call({"operation": "observe", "batch_id": batch}) == release,
                "prepared test batch terminal state could not be confirmed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--vsock", type=Path)
    parser.add_argument("--vm-netns")
    parser.add_argument("--n", type=int)
    parser.add_argument("--sdk-child", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.sdk_child:
        print(json.dumps(sdk_probe()))
        return
    if any(value is None for value in (args.receipt, args.vsock, args.vm_netns, args.n)):
        parser.error("--receipt, --vsock, --vm-netns, and --n are all required")
    print(json.dumps(exercise(args)))


if __name__ == "__main__":
    main()
