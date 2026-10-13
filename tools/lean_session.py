"""Persistent SDK client for the project's real lean-lsp-mcp server.

Use when the desktop tool palette has not yet reloaded its MCP configuration.
One JSON request per line; {"tool": "list"} lists schemas, {"tool": "exit"}
closes the server. This is transport/logging only, not a proof evaluator.
"""

import asyncio
import hashlib
import importlib.metadata
import json
from pathlib import Path
import sys
import time

from mcp import ClientSession, StdioServerParameters, stdio_client

ROOT = Path(__file__).resolve().parents[1]
LOG = ROOT / "artifacts" / "lean-session.jsonl"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None


def revision(arguments):
    path = (ROOT / arguments.get("file_path", "")).resolve()
    if not path.is_relative_to(ROOT):
        raise ValueError("Proof files must remain inside this project.")
    sources = sorted((ROOT / "SandboxModel").rglob("*.lean"))
    sources += sorted((ROOT / "verification/aeneas").rglob("*.lean"))
    fingerprint = hashlib.sha256()
    for source in sources:
        fingerprint.update(str(source.relative_to(ROOT)).encode())
        fingerprint.update(source.read_bytes())
    return {
        "file_sha256": digest(path),
        "sources_sha256": fingerprint.hexdigest(),
        "toolchain": (ROOT / "lean-toolchain").read_text().strip(),
        "manifest_sha256": digest(ROOT / "lake-manifest.json"),
        "server_version": importlib.metadata.version("lean-lsp-mcp"),
        "client_version": importlib.metadata.version("leanclient"),
        "mcp_version": importlib.metadata.version("mcp"),
    }


async def main():
    params = StdioServerParameters(
        command="/bin/bash", args=[str(ROOT / "scripts/lean-mcp-server.sh")], cwd=ROOT
    )
    async with stdio_client(params) as streams:
        async with ClientSession(*streams, read_timeout_seconds=240) as session:
            await session.initialize()
            print(json.dumps({"ready": True, "server": "lean-lsp-mcp"}), flush=True)
            while line := await asyncio.to_thread(sys.stdin.readline):
                if not line.strip():
                    continue
                request = json.loads(line)
                if request["tool"] == "exit":
                    break
                arguments = request.get("arguments", {})
                version_arguments = arguments
                if "code_file" in request:
                    code_path = (ROOT / request["code_file"]).resolve()
                    if not code_path.is_relative_to(ROOT):
                        raise ValueError("Code snippets must remain inside this project.")
                    arguments = {"code": code_path.read_text()}
                    version_arguments = {"file_path": request["code_file"]}
                before = revision(version_arguments)
                expected = request.get("expected_sha256", before["file_sha256"])
                started = time.monotonic()
                print(json.dumps({"started": request["tool"], "file_sha256": before["file_sha256"]}), flush=True)
                if expected != before["file_sha256"]:
                    output = {"error": "File changed since the selected proof state; inspect again."}
                else:
                    try:
                        if request["tool"] == "lean_multi_attempt" and any(
                            "\n" in snippet for snippet in arguments.get("snippets", [])
                        ):
                            raise ValueError("Use single-line tactic trials; use lean_run_code for multi-line proofs.")
                        if request["tool"] == "list":
                            result = await session.list_tools()
                        else:
                            result = await session.call_tool(request["tool"], arguments)
                        output = result.model_dump(mode="json", by_alias=True, exclude_none=True)
                    except Exception as error:
                        output = {"error": str(error)}
                after = revision(version_arguments)
                record = {
                    "request": request, "revision": before,
                    "stale": before != after,
                    "elapsed_seconds": round(time.monotonic() - started, 3),
                    "result": output,
                }
                LOG.parent.mkdir(exist_ok=True)
                with LOG.open("a") as log:
                    log.write(json.dumps(record, ensure_ascii=False) + "\n")
                # Structured results duplicate the text payload; print just one.
                if "structuredContent" in output:
                    record["result"] = output["structuredContent"]
                print(json.dumps(record, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    asyncio.run(main())
