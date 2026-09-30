"""Bounded durable self-service build journal. No untrusted archive is opened here."""
import fcntl
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import sqlite3
import stat
import threading
import time

if __package__:
    from .template_stage import private_directory
else:
    from template_stage import private_directory

MIB = 1024 * 1024
TERMINAL = {"Ready", "Failed"}


class BuildError(Exception):
    def __init__(self, code, reason):
        self.code, self.reason = code, reason
        super().__init__(reason)


def public_image(value):
    # No Dockerfile injection, credentials, local transports or custom ports.
    # Host packet rules, not this lexical check, enforce private-address denial.
    if (not isinstance(value, str) or len(value) > 256
            or not re.fullmatch(r"[a-z0-9][a-z0-9./_-]*(?::[A-Za-z0-9_.-]+|@sha256:[0-9a-f]{64})?", value)
            or ".." in value or "//" in value):
        raise BuildError(400, "InvalidPublicImage")
    first = value.split("/", 1)[0]
    if "/" in value and ("." in first or first == "localhost"):
        try:
            ipaddress.ip_address(first)
        except ValueError:
            if first == "localhost" or first.endswith((".localhost", ".local", ".internal")):
                raise BuildError(400, "InvalidPublicImage") from None
        else:
            raise BuildError(400, "InvalidPublicImage") from None
    return value


def identity(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{32}", value):
        raise BuildError(404, "NotFound")
    return value


def durable_write(path, blocks, limit, *, exact=None):
    """Private supervisor-owned destination. fsync before journal checkpoint."""
    temporary = path.with_suffix(".partial")
    count, digest = 0, hashlib.sha256()
    try:
        with temporary.open("wb") as output:
            os.chmod(temporary, 0o600)
            for block in blocks:
                count += len(block)
                if count > limit:
                    raise BuildError(413, "InputOrArtifactTooLarge")
                digest.update(block)
                output.write(block)
            if exact is not None and count != exact:
                raise BuildError(400, "IncompleteInputOrArtifact")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        return count, digest.hexdigest()
    finally:
        temporary.unlink(missing_ok=True)


class BuildStore:
    def __init__(self, root, limits, *, clock=time.time, free_bytes=None, binding=None):
        self.root, self.limits, self.clock = Path(root), dict(limits), clock
        private_directory(self.root)
        for name in ("lease", "builds.db", "builds.db-wal", "builds.db-shm"):
            path = self.root / name
            if path.exists() or path.is_symlink():
                meta = path.lstat()
                if not stat.S_ISREG(meta.st_mode) or meta.st_nlink != 1 or meta.st_uid != os.geteuid() or meta.st_mode & 0o077:
                    raise ValueError("unsafe build journal file")
        self.free_bytes = free_bytes or (lambda: shutil.disk_usage(self.root).free)
        self.lock = threading.RLock()
        self.uploads = set()
        self.lease = (self.root / "lease").open("a+b")
        os.chmod(self.root / "lease", 0o600)
        try:
            fcntl.flock(self.lease.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            self.lease.close()
            raise BuildError(409, "BuildStoreAlreadyOpen") from None
        for field, value in self.limits.items():
            if type(value) is not int or value <= 0:
                self.lease.close()
                raise ValueError("positive build limit required: " + field)
        self.db = sqlite3.connect(self.root / "builds.db", check_same_thread=False)
        os.chmod(self.root / "builds.db", 0o600)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA synchronous=FULL")
        self.db.execute("CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY, limits TEXT NOT NULL)")
        encoded = json.dumps({"limits": self.limits, "binding": binding}, sort_keys=True)
        old = self.db.execute("SELECT limits FROM settings WHERE id=1").fetchone()
        if old and old[0] != encoded:
            self.close()
            raise ValueError("build budgets changed; explicit migration required")
        self.db.execute("INSERT OR IGNORE INTO settings VALUES(1, ?)", (encoded,))
        self.db.execute("""CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY, owner INTEGER NOT NULL, token TEXT NOT NULL, request TEXT NOT NULL,
            state TEXT NOT NULL, created REAL NOT NULL, deadline REAL NOT NULL,
            reserved INTEGER NOT NULL, input_hash TEXT, candidate TEXT, failure TEXT, result TEXT,
            UNIQUE(owner,token))""")
        self.db.commit()

    def close(self):
        if hasattr(self, "db"):
            self.db.close()
        self.lease.close()

    def directory(self, job):
        return self.root / identity(job)

    def validate(self, request):
        if not isinstance(request, dict) or set(request) - {"kind", "image", "resources", "timeoutSeconds", "allowInternetAccess"}:
            raise BuildError(400, "InvalidBuildRequest")
        kind = request.get("kind")
        if kind not in ("dockerfile", "oci") or (kind == "oci") != ("image" in request):
            raise BuildError(400, "InvalidBuildRequest")
        resources = request.get("resources", {})
        if not isinstance(resources, dict) or set(resources) - {"cpuCount", "memoryMiB", "diskMiB", "minFreeMiB"}:
            raise BuildError(400, "InvalidResources")
        values = {"cpuCount": 1, "memoryMiB": 256, "diskMiB": 1024, "minFreeMiB": 64} | resources
        for field, lower, upper in (("cpuCount", 1, self.limits["max_cpu"]),
                ("memoryMiB", 128, self.limits["max_memory_mib"]),
                ("diskMiB", 512, self.limits["max_disk_mib"]), ("minFreeMiB", 16, self.limits["max_disk_mib"])):
            if type(values[field]) is not int or not lower <= values[field] <= upper:
                raise BuildError(400, "InvalidResources")
        if values["minFreeMiB"] >= values["diskMiB"]:
            raise BuildError(400, "InvalidResources")
        timeout = request.get("timeoutSeconds", self.limits["timeout_seconds"])
        internet = request.get("allowInternetAccess", True)
        if type(timeout) is not int or not 1 <= timeout <= self.limits["timeout_seconds"] or type(internet) is not bool:
            raise BuildError(400, "InvalidBuildRequest")
        if kind == "oci" and not internet:
            raise BuildError(400, "PublicImageNeedsNetwork")
        result = {"kind": kind, "resources": values, "timeoutSeconds": timeout, "allowInternetAccess": internet}
        if kind == "oci":
            result["image"] = public_image(request["image"])
        return result

    def submit(self, owner, request, token=None):
        request = self.validate(request)
        token = token or secrets.token_hex(16)
        if not isinstance(token, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", token):
            raise BuildError(400, "InvalidIdempotencyKey")
        encoded = json.dumps(request, sort_keys=True)
        with self.lock, self.db:
            old = self.db.execute("SELECT * FROM jobs WHERE owner=? AND token=?", (owner, token)).fetchone()
            if old:
                if old["request"] != encoded:
                    raise BuildError(409, "IdempotencyConflict")
                return self.view(old)
            counts = self.db.execute("SELECT COUNT(*), COALESCE(SUM(reserved),0) FROM jobs").fetchone()
            own = self.db.execute("SELECT COUNT(*), COALESCE(SUM(reserved),0) FROM jobs WHERE owner=?", (owner,)).fetchone()
            active = self.db.execute("SELECT COUNT(*) FROM jobs WHERE reserved>0").fetchone()[0]
            own_active = self.db.execute("SELECT COUNT(*) FROM jobs WHERE owner=? AND reserved>0", (owner,)).fetchone()[0]
            # Independent VM disk + incoming material + two opaque candidate
            # copies (download/staging), kernel/log/journal margin. Failed and
            # unknown jobs retain this reservation until exact cleanup.
            cost = (self.limits["builder_disk_bytes"] + self.limits["max_input_bytes"]
                    + 2 * request["resources"]["diskMiB"] * MIB + self.limits["artifact_margin_bytes"])
            # Terminal jobs retain bounded logs and journal/status. Charge
            # their worst-case bytes even after the active VM budget releases.
            history = self.limits["max_log_bytes"] + 256 * 1024
            if (counts[0] >= self.limits["max_jobs"] or own[0] >= self.limits["max_jobs_per_tenant"]
                    or active >= self.limits["max_pending"] or own_active >= self.limits["max_pending_per_tenant"]
                    or counts[1] + cost + (counts[0] + 1) * history > self.limits["workspace_bytes"]
                    or own[1] + cost + (own[0] + 1) * history > self.limits["workspace_bytes_per_tenant"]
                    or self.free_bytes() < counts[1] + cost + (counts[0] + 1) * history + self.limits["runtime_cleanup_reserve_bytes"]):
                raise BuildError(429, "BuildBudgetExceeded")
            job = secrets.token_hex(16)
            self.directory(job).mkdir(mode=0o700)
            now = self.clock()
            self.db.execute("INSERT INTO jobs VALUES(?,?,?,?,?,?,?,?,NULL,NULL,NULL,NULL)",
                (job, owner, token, encoded, "AwaitingInput" if request["kind"] == "dockerfile" else "Queued",
                 now, now + request["timeoutSeconds"], cost))
            return self.view(self._row(job, owner))

    def _row(self, job, owner=None):
        identity(job)
        row = self.db.execute("SELECT * FROM jobs WHERE id=?", (job,)).fetchone()
        if row is None or (owner is not None and row["owner"] != owner):
            raise BuildError(404, "NotFound")
        return row

    def get(self, job, owner=None):
        with self.lock:
            return dict(self._row(job, owner))

    def view(self, row):
        return {"buildID": row["id"], "state": row["state"], "failure": row["failure"],
                "createdAt": row["created"], "deadline": row["deadline"],
                "template": json.loads(row["result"]) if row["result"] else None}

    def query(self, job, owner):
        with self.lock:
            return self.view(self._row(job, owner))

    def change(self, job, state, *, failure=None, candidate=None, result=None, release=False):
        with self.lock, self.db:
            row = self._row(job)
            if row["state"] in TERMINAL:
                raise BuildError(409, "TerminalBuild")
            self.db.execute("UPDATE jobs SET state=?, failure=?, candidate=COALESCE(?,candidate), result=?, reserved=? WHERE id=?",
                (state, failure, json.dumps(candidate) if candidate else None,
                 json.dumps(result) if result else None, 0 if release else row["reserved"], job))

    def upload(self, job, owner, blocks, size):
        with self.lock:
            row = self._row(job, owner)
            if row["state"] != "AwaitingInput" or job in self.uploads:
                raise BuildError(409, "InputAlreadyCommittedOrUploading")
            if type(size) is not int or not 1 <= size <= self.limits["max_input_bytes"]:
                raise BuildError(413, "InputTooLarge")
            if self.clock() >= row["deadline"]:
                raise BuildError(409, "BuildDeadlineExceeded")
            self.uploads.add(job)
        try:
            _, digest = durable_write(self.directory(job) / "source.tar", blocks, size, exact=size)
            with self.lock, self.db:
                self.db.execute("UPDATE jobs SET state='Queued',input_hash=? WHERE id=? AND state='AwaitingInput'", (digest, job))
        finally:
            with self.lock:
                self.uploads.discard(job)

    def rows(self):
        with self.lock:
            return [dict(row) for row in self.db.execute("SELECT * FROM jobs ORDER BY created,id")]

    def logs(self, job, owner, offset, limit):
        self.get(job, owner)
        if type(offset) is not int or offset < 0 or type(limit) is not int or not 1 <= limit <= 65536:
            raise BuildError(400, "InvalidLogRange")
        path = self.directory(job) / "build.log"
        data = b""
        if path.exists():
            with path.open("rb") as incoming:
                incoming.seek(min(offset, path.stat().st_size))
                data = incoming.read(limit)
        return {"offset": offset, "nextOffset": offset + len(data), "text": data.decode("utf-8", errors="replace")}
