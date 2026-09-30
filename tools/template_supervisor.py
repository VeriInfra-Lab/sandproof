#!/usr/bin/env python3
"""Opt-in, serial trusted supervisor for the durable build inbox.

Run on the root-owned KVM controller host, with a separate builder-only tenant.
Never execute input or unpack an image here. A claimed job is never retried;
interrupted/uncertain jobs retain their identity and block further dispatch.
"""
import argparse
import fcntl
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shutil
import sqlite3
import stat
import sys

if __package__:
    from .template_stage import private_directory, stage
else:
    from template_stage import private_directory, stage


def protected_file(path):
    path = Path(path)
    meta = path.lstat()
    if (path.resolve() != path or not stat.S_ISREG(meta.st_mode)
            or meta.st_uid != os.geteuid() or meta.st_mode & 0o022 or meta.st_nlink != 1):
        raise ValueError("protected regular file required")
    return path


class Queue:
    def __init__(self, root, max_log_bytes):
        self.root = Path(root)
        private_directory(self.root)
        self.db = sqlite3.connect(protected_file(self.root / "builds.db"), timeout=5)
        self.db.execute("PRAGMA synchronous=FULL")
        self.db.execute("PRAGMA trusted_schema=OFF")
        if (self.db.execute("PRAGMA application_id").fetchone()[0] != 1397768770
                or self.db.execute("PRAGMA user_version").fetchone()[0] != 1):
            raise ValueError("unsupported build inbox")
        if type(max_log_bytes) is not int or not 1 <= max_log_bytes <= 1024 * 1024:
            raise ValueError("invalid log limit")
        self.max_log_bytes = max_log_bytes

    def claim(self):
        self.db.execute("BEGIN IMMEDIATE")
        try:
            if self.db.execute("SELECT 1 FROM builds WHERE state='building'").fetchone():
                raise RuntimeError("unresolved build: inspect original identity before dispatch")
            row = self.db.execute("SELECT owner,id,request,input,digest FROM builds WHERE state='queued' ORDER BY rowid LIMIT 1").fetchone()
            if row is None:
                self.db.commit()
                return None
            owner, build_id, request, source, digest = row
            if hashlib.sha256(source).hexdigest() != digest:
                raise ValueError("stored build input changed")
            claim = secrets.token_hex(16)
            self.db.execute("UPDATE builds SET state='building',claim=? WHERE owner=? AND id=? AND state='queued'",
                            (claim, owner, build_id))
            self.db.commit()
            return {"owner": owner, "id": build_id, "request": json.loads(request), "source": source, "claim": claim}
        except BaseException:
            self.db.rollback()
            raise

    def update(self, job, sql, values):
        with self.db:
            changed = self.db.execute(sql + " WHERE owner=? AND id=? AND claim=? AND state='building'",
                                      (*values, job["owner"], job["id"], job["claim"])).rowcount
            if changed != 1:
                raise RuntimeError("stale or terminal build receipt")

    def resource(self, job, sandbox_id):
        # Write before uploading input or issuing any build command. Do not
        # overwrite a recorded identity even if a caller invents a new VM.
        with self.db:
            changed = self.db.execute("UPDATE builds SET resource=? WHERE owner=? AND id=? AND claim=? AND state='building' AND resource IS NULL",
                (sandbox_id, job["owner"], job["id"], job["claim"])).rowcount
            if changed != 1:
                raise RuntimeError("build resource already bound or stale")

    def log(self, job, data, truncated=False):
        self.update(job, "UPDATE builds SET log=?,truncated=?", (bytes(data[:self.max_log_bytes]),
                    truncated or len(data) > self.max_log_bytes))

    def finish(self, job, *, candidate=None, failure=None):
        if (candidate is None) == (failure is None):
            raise ValueError("one terminal outcome required")
        if candidate is not None and (len(candidate) != 32 or any(c not in "0123456789abcdef" for c in candidate)):
            raise ValueError("invalid candidate")
        if failure is not None and failure not in ("build_failed", "candidate_rejected"):
            raise ValueError("bounded public failure code required")
        self.update(job, "UPDATE builds SET state=?,candidate=?,failure=?",
                    ("candidate" if candidate else "failed", candidate, failure))


def download(sandbox, guest_path, destination, limit):
    total = 0
    with sandbox.files.read(guest_path, format="stream", user="root", request_timeout=180) as stream:
        with destination.open("xb") as output:
            for block in stream:
                total += len(block)
                if total > limit:
                    raise ValueError("guest output exceeds budget")
                output.write(block)
            output.flush()
            os.fsync(output.fileno())
    return total


def run_one(queue, job, settings, profile, builder_tenant, builder_template, url):
    from e2b import Sandbox
    from urllib.parse import urlsplit
    if importlib.metadata.version("e2b") != "2.51.0":
        raise RuntimeError("pinned E2B SDK 2.51.0 required")
    endpoint = urlsplit(url)
    if endpoint.scheme != "http" or endpoint.hostname not in ("127.0.0.1", "localhost", "::1"):
        raise ValueError("supervisor requires a local controller endpoint")
    if str(builder_tenant) == job["owner"]:
        raise ValueError("builder tenant must be separate from the submitting tenant")
    key = next(c["secret"] for c in settings["credentials"] if c["tenant"] == builder_tenant)
    required = {"buildctl", "buildkitd", "buildkit-runc", "umoci", "busybox", "envd"}
    if set(profile) != required:
        raise ValueError("complete pinned tool profile required")
    for artifact in profile.values():
        source = protected_file(Path(artifact["path"]))
        with source.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if digest != artifact["sha256"]:
            raise ValueError("tool fingerprint mismatch")
    root = queue.root / (job["owner"] + "-" + job["id"] + "-" + job["claim"])
    root.mkdir(mode=0o700)
    request = job["request"]
    if shutil.disk_usage(root).free < 2 * request["disk_mib"] * 1024 * 1024 + 1024**3:
        queue.finish(job, failure="build_failed")
        root.rmdir()
        return
    sandbox, cleaned, candidate, succeeded = None, False, None, False
    try:
        # Admission is durable before this call. An unknown create reply keeps
        # the job building; its metadata allows an operator to locate it.
        sandbox = Sandbox.create(template=builder_template, timeout=900, allow_internet_access=False,
            metadata={"sandproof-build": job["id"], "sandproof-claim": job["claim"]},
            api_key=key, api_url=url, sandbox_url=url, domain="example.invalid", retries=0, request_timeout=180)
        queue.resource(job, sandbox.sandbox_id)
        def command(text, timeout=60):
            result = sandbox.commands.run(text, user="root", timeout=timeout)
            if result.exit_code != 0:
                raise RuntimeError("build tool failed")
            return result.stdout
        command("mkdir -p /input/tools /output")
        guest_profile = {}
        for name, artifact in profile.items():
            with Path(artifact["path"]).open("rb") as stream:
                sandbox.files.write("/input/tools/" + name, stream, user="root", request_timeout=180)
            guest_profile[name] = {"path": "/input/tools/" + name, "sha256": artifact["sha256"]}
        command("chmod 755 /input/tools/*")
        for name in ("template_worker.py", "template_inputs.py"):
            with Path(__file__).with_name(name).open("rb") as stream:
                sandbox.files.write("/input/" + name, stream, user="root")
        sandbox.files.write("/input/source.tar", job["source"], user="root", request_timeout=180)
        sandbox.files.write("/input/profile.json", json.dumps(guest_profile), user="root")
        worker_request = {"kind": request["kind"], "disk_mib": request["disk_mib"], "min_free_mib": request["min_free_mib"],
            "max_input_bytes": 16 * 1024**2, "max_expanded_bytes": 512 * 1024**2, "max_entries": 10000,
            "timeout_seconds": 300, "max_log_bytes": queue.max_log_bytes}
        sandbox.files.write("/input/request.json", json.dumps(worker_request), user="root")
        # Boot ID is read and passed inside the guest; no user text becomes a
        # host shell command or command argument.
        command('python3 /input/template_worker.py --work /output/job --source /input/source.tar '
                '--request /input/request.json --profile /input/profile.json '
                '--guest-boot-id "$(cat /proc/sys/kernel/random/boot_id)"', timeout=330)
        download(sandbox, "/output/job/status.json", root / "status.json", 65536)
        status = json.loads((root / "status.json").read_bytes())
        download(sandbox, "/output/job/candidate.ext4", root / "candidate.ext4", request["disk_mib"] * 1024**2)
        succeeded = True
    except Exception:
        # Raw exception strings can contain credentials/host paths. Keep a
        # bounded public category; physical uncertainty remains building.
        succeeded = False
    finally:
        if sandbox is not None:
            try:
                log = bytearray()
                truncated = False
                for name in ("build.log", "daemon.log"):
                    try:
                        with sandbox.files.read("/output/job/" + name, format="stream", user="root", request_timeout=20) as stream:
                            for block in stream:
                                remaining = queue.max_log_bytes - len(log)
                                log.extend(block[:remaining])
                                if len(block) > remaining:
                                    truncated = True
                                    break
                    except Exception:
                        pass
                queue.log(job, log, truncated)
            finally:
                # kill returning success is the controller's exact cleanup
                # confirmation. Unknown replies must retain the claim.
                cleaned = sandbox.kill(request_timeout=180)
    if not cleaned:
        raise RuntimeError("build outcome/cleanup unknown; retain original claim")
    if succeeded:
        try:
            candidate = stage(Path(settings["template_registry"]["root"]), Path(settings["kernel"]),
                root / "candidate.ext4", status, int(job["owner"]), request["cpu_count"],
                request["memory_mib"], request["host_memory_mib"])
        except Exception:
            queue.finish(job, failure="candidate_rejected")
        else:
            queue.finish(job, candidate=candidate)
    else:
        queue.finish(job, failure="build_failed")
    # Only this exact claim directory is removed, after authoritative VM cleanup
    # and the durable terminal receipt. Published/staged artifacts are separate.
    shutil.rmtree(root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--builder-tenant", type=int, required=True)
    parser.add_argument("--builder-template", required=True)
    parser.add_argument("--url", required=True)
    args = parser.parse_args()
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("isolated root Linux/KVM host required")
    settings = json.loads(protected_file(args.config).read_bytes())
    profile = json.loads(protected_file(args.profile).read_bytes())
    config = settings["template_builds"]
    queue = Queue(config["root"], config["max_log_bytes"])
    lock_path = queue.root / "supervisor.lock"
    fd = os.open(lock_path, os.O_CREAT | os.O_NOFOLLOW | os.O_WRONLY, 0o600)
    with os.fdopen(fd, "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        job = queue.claim()
        if job is not None:
            run_one(queue, job, settings, profile, args.builder_tenant, args.builder_template, args.url)


if __name__ == "__main__":
    main()
