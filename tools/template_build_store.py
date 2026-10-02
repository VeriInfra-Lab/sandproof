"""Bounded durable self-service build journal. No untrusted archive is opened here."""
import contextlib
import fcntl
import math
import hashlib
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
    from .template_stage import private_directory, write_sparse
    from .template_fetch import reference
    from .template_inputs import InvalidTemplate
    from .template_build_windows import WindowHelper, WindowReducer, WindowRejected
    from .template_physical_storage import PhysicalProfileLease
else:
    from template_stage import private_directory, write_sparse
    from template_fetch import reference
    from template_inputs import InvalidTemplate
    from template_build_windows import WindowHelper, WindowReducer, WindowRejected
    from template_physical_storage import PhysicalProfileLease

MIB = 1024 * 1024
TERMINAL = {"Ready", "Failed"}


class BuildError(Exception):
    def __init__(self, code, reason):
        self.code, self.reason = code, reason
        super().__init__(reason)


def public_image(value):
    try:
        repository, identifier = reference(value)
    except InvalidTemplate:
        raise BuildError(400, "InvalidPublicImage") from None
    return repository + ("@" if identifier.startswith("sha256:") else ":") + identifier


def identity(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{32}", value):
        raise BuildError(404, "NotFound")
    return value


def owned_input(path, identity):
    """Bind an upload path to its private single-link inode."""
    try:
        meta = path.lstat()
    except FileNotFoundError:
        return False
    if (not stat.S_ISREG(meta.st_mode) or (meta.st_dev, meta.st_ino) != identity
            or meta.st_nlink != 1 or meta.st_uid != os.geteuid()):
        raise BuildError(503, "InputCleanupIdentityUnknown")
    return True


def remove_owned_input(path, identity):
    """Never let upload cancellation adopt a replacement directory entry."""
    if not owned_input(path, identity):
        return
    try:
        path.unlink()
        fsync_directory(path.parent)
    except OSError:
        raise BuildError(503, "InputCleanupUnknown") from None


def durable_write(path, blocks, limit, *, exact=None, sparse=False, check_deadline=None, on_publish=None):
    """Private supervisor-owned destination. fsync before journal checkpoint."""
    temporary = path.with_suffix(".partial")
    count, digest = 0, hashlib.sha256()
    owned, published = None, False
    def check():
        if check_deadline is not None:
            check_deadline()
    try:
        check()
        if check_deadline is not None and (path.exists() or path.is_symlink()):
            raise BuildError(503, "InputCleanupIdentityUnknown")
        # Deadline-bound uploads never adopt another invocation's partial.
        with temporary.open("xb" if check_deadline is not None else "wb") as output:
            meta = os.fstat(output.fileno())
            owned = meta.st_dev, meta.st_ino
            os.fchmod(output.fileno(), 0o600)
            for block in blocks:
                check()
                count += len(block)
                if count > limit:
                    raise BuildError(413, "InputOrArtifactTooLarge")
                digest.update(block)
                if sparse:
                    write_sparse(output, block)
                else:
                    output.write(block)
            check()  # The last block or EOF may have arrived after expiry.
            if exact is not None and count != exact:
                raise BuildError(400, "IncompleteInputOrArtifact")
            if sparse:
                output.truncate(count)
            output.flush()
            os.fsync(output.fileno())
            check()
        if check_deadline is not None:
            if not owned_input(temporary, owned) or path.exists() or path.is_symlink():
                raise BuildError(503, "InputCleanupIdentityUnknown")
        os.replace(temporary, path)
        published = True
        if on_publish is not None:
            on_publish(owned)
        descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        check()
        return count, digest.hexdigest()
    except FileExistsError:
        if check_deadline is not None:
            raise BuildError(503, "InputCleanupIdentityUnknown") from None
        raise
    except BaseException:
        if check_deadline is not None and published:
            remove_owned_input(path, owned)
        raise
    finally:
        if check_deadline is None:
            temporary.unlink(missing_ok=True)
        elif owned is not None and not published:
            remove_owned_input(temporary, owned)


def fsync_directory(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def bounded_configuration(value):
    fields = {"namespace", "windows", "retention_ms", "max_request_bytes", "max_result_bytes",
              "max_record_bytes", "database_bytes", "wal_bytes", "recovery_reserve_bytes", "helper"}
    if type(value) is not dict or set(value) != fields:
        raise ValueError("explicit complete bounded_builds configuration required")
    for name in fields - {"windows", "helper"}:
        if type(value[name]) is not int or not 0 < value[name] < 2**63:
            raise ValueError("invalid bounded_builds " + name)
    windows = value["windows"]
    if type(windows) is not dict or set(windows) != {"max_open_windows", "max_duration_ms"}:
        raise ValueError("invalid build window limits")
    if any(type(v) is not int or not 0 < v < 2**63 for v in windows.values()):
        raise ValueError("invalid build window limits")
    if (value["max_record_bytes"] < value["max_request_bytes"] + value["max_result_bytes"] + 4096
            or value["database_bytes"] < 65536
            or value["wal_bytes"] < 2 * value["database_bytes"] + 65536
            or value["recovery_reserve_bytes"] < value["database_bytes"]):
        raise ValueError("insufficient encoded record or SQLite transaction reserve")
    if type(value["helper"]) is not dict:
        raise ValueError("explicit window helper required")
    helper = WindowHelper(**value["helper"])
    framing = 512 + 256 * windows["max_open_windows"]
    if (helper.max_input_bytes < framing or helper.max_output_bytes < framing + 256
            or value["max_request_bytes"] > 65536 or value["max_result_bytes"] > 65536 - 4096):
        raise ValueError("bounded build limits exceed window or HTTP framing budget")
    return value


class BuildStore:
    def __init__(self, root, limits, *, clock=time.time, free_bytes=None, binding=None,
                 bounded=None, reducer=None, migrate_legacy=False, fault=None, physical_profile=None):
        self.root, self.limits, self.clock = Path(root), dict(limits), clock
        self.bounded = bounded_configuration(bounded) if bounded is not None else None
        self.reducer = (reducer or WindowReducer(WindowHelper(**bounded["helper"]))) if bounded else None
        self.fault = fault or (lambda _point: None)
        self.physical_lease = None
        if physical_profile is not None:
            if self.bounded is not None and self.limits["workspace_bytes"] < sum(
                    self.bounded[field] for field in ("database_bytes", "wal_bytes", "recovery_reserve_bytes")):
                raise ValueError("physical workspace must contain the complete public journal reserve")
            public = physical_profile.get("public_build")
            if (self.bounded is None or not public or public["root"] != str(self.root)
                    or public["workspace_bytes"] != self.limits["workspace_bytes"]
                    or public["max_jobs"] != self.limits["max_jobs"]):
                raise ValueError("physical profile requires its exact bounded build root and budgets")
            self.physical_lease = PhysicalProfileLease(physical_profile)
            if (self.limits.get("physical_storage_peak_bytes") != self.physical_lease.peak
                    or self.limits.get("physical_storage_inodes") != self.physical_lease.inodes):
                self.physical_lease.close()
                raise ValueError("physical admission limits differ from immutable profile")
        elif "physical_storage_peak_bytes" in self.limits or "physical_storage_inodes" in self.limits:
            raise ValueError("physical limits require a validated activation lease")
        private_directory(self.root)
        for name in ("lease", "builds.db", "builds.db-wal", "builds.db-shm"):
            path = self.root / name
            if path.exists() or path.is_symlink():
                meta = path.lstat()
                if not stat.S_ISREG(meta.st_mode) or meta.st_nlink != 1 or meta.st_uid != os.geteuid() or meta.st_mode & 0o077:
                    raise ValueError("unsafe build journal file")
                if self.bounded and name in ("builds.db", "builds.db-wal"):
                    quota = self.bounded["database_bytes" if name == "builds.db" else "wal_bytes"]
                    if meta.st_size > quota:
                        raise ValueError("existing build journal exceeds bounded file quota")
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
        try:
            for field, value in self.limits.items():
                if type(value) is not int or value <= 0:
                    raise ValueError("positive build limit required: " + field)
            self.db = sqlite3.connect(self.root / "builds.db", check_same_thread=False)
            os.chmod(self.root / "builds.db", 0o600)
            self.db.row_factory = sqlite3.Row
            self.db.execute("PRAGMA journal_mode=WAL")
            self.db.execute("PRAGMA synchronous=FULL")
            self.db.execute("PRAGMA temp_store=MEMORY")
            self.db.execute("PRAGMA wal_autocheckpoint=1")
            self.db.execute("PRAGMA journal_size_limit=0")
            if self.bounded:
                page = self.db.execute("PRAGMA page_size").fetchone()[0]
                minimum = (self.limits["max_jobs"] * (4 * self.bounded["max_record_bytes"] + 4096)
                           + self.bounded["windows"]["max_open_windows"] * 512 + 65536)
                if self.bounded["database_bytes"] < minimum:
                    raise ValueError("database quota cannot reserve all accepted future records")
                pages = self.bounded["database_bytes"] // page
                if self.db.execute("PRAGMA page_count").fetchone()[0] > pages:
                    raise ValueError("existing build database exceeds bounded quota")
                self.db.execute("PRAGMA max_page_count=" + str(pages))
                checkpoint = self.db.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
                if checkpoint[0] or checkpoint[1] != checkpoint[2]:
                    raise BuildError(503, "BuildCheckpointBlocked")
                if self.free_bytes() < self.bounded["wal_bytes"] + self.bounded["recovery_reserve_bytes"]:
                    raise BuildError(503, "BuildJournalReserveUnavailable")
            self.db.execute("CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY, limits TEXT NOT NULL)")
            settings = {"limits": self.limits, "binding": binding}
            legacy_encoded = json.dumps(settings, sort_keys=True)
            previous_encoded = json.dumps(settings | {"schema_version": 2}, sort_keys=True)
            encoded = json.dumps(settings | ({"schema_version": 4, "bounded_builds": self.bounded}
                                             if bounded else {"schema_version": 2}), sort_keys=True)
            old = self.db.execute("SELECT limits FROM settings WHERE id=1").fetchone()
            if self.bounded:
                unsafe_random_format = json.dumps(settings | {"schema_version": 3, "bounded_builds": self.bounded}, sort_keys=True)
                if old and old[0] == unsafe_random_format:
                    # Format 3 could delete random IDs. Surviving rows cannot
                    # reconstruct all identities ever exposed to clients.
                    raise ValueError("bounded random-ID journal lacks a permanent build frontier")
                if old is None and self.db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='jobs'").fetchone():
                    raise ValueError("existing build journal is missing its identity configuration")
            if old and old[0] != encoded:
                if old[0] not in (legacy_encoded, previous_encoded) or (bounded and not migrate_legacy):
                    raise ValueError("build budgets changed; explicit migration required")
            self.db.execute("BEGIN IMMEDIATE")
            self.db.execute("INSERT OR REPLACE INTO settings VALUES(1, ?)", (encoded,))
            self.db.execute("""CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY, owner TEXT NOT NULL, token TEXT NOT NULL, request TEXT NOT NULL,
                state TEXT NOT NULL, created REAL NOT NULL, deadline REAL NOT NULL,
                reserved INTEGER NOT NULL, input_hash TEXT, candidate TEXT, failure TEXT, result TEXT,
                publication_attempted INTEGER NOT NULL DEFAULT 0, UNIQUE(owner,token))""")
            columns = {row[1] for row in self.db.execute("PRAGMA table_info(jobs)")}
            if "publication_attempted" not in columns:
                self.db.execute("ALTER TABLE jobs ADD COLUMN publication_attempted INTEGER NOT NULL DEFAULT 0")
                self.db.execute("UPDATE jobs SET publication_attempted=1 WHERE state='Publishing'")
            elif old and old[0] == legacy_encoded:
                self.db.execute("UPDATE jobs SET publication_attempted=1 WHERE state='Publishing'")
            extras = {"window_namespace": "TEXT NOT NULL DEFAULT ''", "window_sequence": "TEXT NOT NULL DEFAULT ''",
                      "fs_phase": "TEXT NOT NULL DEFAULT 'Legacy'", "fs_nonce": "TEXT", "fs_identity": "TEXT",
                      "delete_manifest": "TEXT", "retained_until_ms": "INTEGER", "private_window": "TEXT",
                      "builder_settled": "TEXT", "private_ack": "INTEGER NOT NULL DEFAULT 0",
                      "staging_cleanup": "TEXT"}
            for name, kind in extras.items():
                if name not in columns:
                    self.db.execute("ALTER TABLE jobs ADD COLUMN " + name + " " + kind)
            if bounded:
                # Rebuild only to remove legacy owner/key uniqueness. The legacy
                # empty namespace remains disjoint and is never remapped.
                sql = self.db.execute("SELECT sql FROM sqlite_master WHERE name='jobs'").fetchone()[0]
                if "UNIQUE(owner,token)" in sql:
                    self.db.execute(sql.replace("CREATE TABLE jobs", "CREATE TABLE jobs_new", 1)
                                    .replace("UNIQUE(owner,token)", "UNIQUE(owner,window_namespace,window_sequence,token)"))
                    self.db.execute("INSERT INTO jobs_new SELECT * FROM jobs")
                    self.db.execute("DROP TABLE jobs")
                    self.db.execute("ALTER TABLE jobs_new RENAME TO jobs")
                self.db.execute("CREATE TABLE IF NOT EXISTS request_windows (id INTEGER PRIMARY KEY CHECK(id=1), state TEXT NOT NULL)")
                initial = {"limits": bounded["windows"], "issued_through": None, "time_floor_ms": 0, "open": []}
                self.db.execute("INSERT OR IGNORE INTO request_windows VALUES(1,?)", (json.dumps(initial),))
                if self.db.execute("SELECT COUNT(*) FROM jobs").fetchone()[0] > limits["max_jobs"]:
                    raise ValueError("legacy jobs exceed bounded migration quota")
                self._initialize_job_allocator(current_format=bool(old and old[0] == encoded))
                for row in self.db.execute("SELECT * FROM jobs"):
                    self._check_record(dict(row))
            self.db.commit()
            self.recover_filesystem()
        except BaseException:
            self.close()
            raise

    def _job_frontier(self):
        row = self.db.execute("SELECT namespace,issued_through FROM job_allocator WHERE id=1").fetchone()
        prefix = f"{self.bounded['namespace']:016x}"
        if row is None or row["namespace"] != prefix:
            raise ValueError("invalid permanent build allocator namespace")
        value = row["issued_through"]
        if value is not None and (type(value) is not str or not re.fullmatch(r"[0-9a-f]{16}", value)):
            raise ValueError("invalid permanent build frontier")
        return None if value is None else int(value, 16)

    def _initialize_job_allocator(self, *, current_format):
        prefix = f"{self.bounded['namespace']:016x}"
        exists = self.db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='job_allocator'").fetchone()
        if current_format:
            # Never reconstruct a lost frontier from surviving post-GC rows.
            if not exists:
                raise ValueError("permanent build frontier is missing")
            frontier = self._job_frontier()
        else:
            if exists:
                raise ValueError("unexpected permanent build allocator in legacy journal")
            # The only permitted source formats never retire legacy IDs. Seed
            # above every retained random ID in this namespace's 64-bit suffix
            # range; different prefixes are permanently disjoint.
            frontier = None
            for row in self.db.execute("SELECT id FROM jobs"):
                job = identity(row["id"])
                if job.startswith(prefix):
                    suffix = int(job[16:], 16)
                    frontier = suffix if frontier is None else max(frontier, suffix)
            self.db.execute("""CREATE TABLE job_allocator (
                id INTEGER PRIMARY KEY CHECK(id=1), namespace TEXT NOT NULL,
                issued_through TEXT CHECK(issued_through IS NULL OR length(issued_through)=16))""")
            self.db.execute("INSERT INTO job_allocator VALUES(1,?,?)",
                            (prefix, None if frontier is None else f"{frontier:016x}"))
        for row in self.db.execute("SELECT id,window_namespace FROM jobs"):
            job = identity(row["id"])
            if row["window_namespace"] and (row["window_namespace"] != str(self.bounded["namespace"])
                                           or not job.startswith(prefix)):
                raise ValueError("job identity disagrees with permanent namespace")
            if job.startswith(prefix) and (frontier is None or int(job[16:], 16) > frontier):
                raise ValueError("job identity exceeds permanent build frontier")

    def _allocate_job(self):
        # Called only inside the same admission transaction as the jobs INSERT.
        # SQLite TEXT preserves the full unsigned u64 space without conversion
        # through signed INTEGER or floating point. Exhaustion never wraps.
        frontier = self._job_frontier()
        if frontier == 2**64 - 1:
            return None
        sequence = 0 if frontier is None else frontier + 1
        self.db.execute("UPDATE job_allocator SET issued_through=? WHERE id=1", (f"{sequence:016x}",))
        self.fault("after_job_allocation")
        return f"{self.bounded['namespace']:016x}{sequence:016x}"

    def close(self):
        if getattr(self, "physical_lease", None) is not None:
            self.physical_lease.close()
        if hasattr(self, "db"):
            self.db.close()
        self.lease.close()

    def directory(self, job):
        return self.root / identity(job)

    @contextlib.contextmanager
    def transaction(self, *, check_before_commit=None):
        # No streaming readers on our sole connection. A foreign held reader
        # may prevent checkpointing; stop writes rather than growing its WAL.
        with self.lock:
            if self.bounded:
                result = self.db.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
                if result[0] or result[1] != result[2]:
                    raise BuildError(503, "BuildCheckpointBlocked")
                if self.free_bytes() < self.bounded["wal_bytes"] + self.bounded["recovery_reserve_bytes"]:
                    raise BuildError(503, "BuildJournalReserveUnavailable")
            self.db.execute("BEGIN IMMEDIATE")
            try:
                yield
                self.fault("before_commit")
                if check_before_commit is not None:
                    check_before_commit()
                self.db.commit()
                self.fault("after_commit")
            except BaseException:
                self.db.rollback()
                raise

    def _check_record(self, row):
        if not self.bounded:
            return
        # Reserve the *future whole encoded record*, not just today's sparse
        # row. JSON text columns are escaped again by this outer encoding.
        # These fixed framing bounds cover all exact fixed-name manifests and
        # protocol identities; request/candidate themselves never grow later.
        projected = dict(row)
        for name, maximum in {"result": 2 * self.bounded["max_result_bytes"],
                              "private_window": 1024, "builder_settled": 2048,
                              "staging_cleanup": 2048, "delete_manifest": 2048,
                              "fs_identity": 128, "failure": 1024, "input_hash": 128}.items():
            actual = row.get(name)
            if actual is not None and len(json.dumps(actual).encode()) > maximum:
                raise BuildError(413, "BuildRecordTooLarge")
            projected[name] = "x" * maximum
        projected.update(state="CleanupUnknown", fs_phase="Preparing", private_ack=1,
                         retained_until_ms=2**63 - 1)
        if max(len(json.dumps(row, sort_keys=True).encode()), len(json.dumps(projected, sort_keys=True).encode())) > self.bounded["max_record_bytes"]:
            raise BuildError(413, "BuildRecordTooLarge")

    def _now_ms(self):
        value = self.clock()
        if not isinstance(value, (int, float)) or not math.isfinite(value) or not 0 <= value < 2**63 / 1000:
            raise BuildError(503, "InvalidBuildClock")
        return int(value * 1000)

    def _window_state(self):
        return json.loads(self.db.execute("SELECT state FROM request_windows WHERE id=1").fetchone()[0])

    def _reduce(self, value):
        decision = self.reducer.reduce(self._window_state(), value)
        self.db.execute("UPDATE request_windows SET state=? WHERE id=1", (json.dumps(decision["next"], sort_keys=True),))
        return decision["event"]

    def _advance(self):
        # Clamped observed time never reverses the persisted trusted floor.
        now = max(self._now_ms(), self._window_state()["time_floor_ms"])
        self._reduce({"AdvanceTime": {"now_ms": now}})
        return now

    def _window(self, window):
        if (type(window) is not dict or set(window) != {"namespace", "sequence"}
                or type(window["namespace"]) is not int or type(window["sequence"]) is not int
                or not 0 <= window["sequence"] < 2**64):
            raise BuildError(400, "InvalidRequestWindow")
        if window["namespace"] != self.bounded["namespace"]:
            raise BuildError(409, "WrongRequestNamespace")
        return str(window["namespace"]), str(window["sequence"])

    def open_window(self, owner, duration_ms):
        if not self.bounded:
            raise BuildError(409, "BuildWindowsDisabled")
        if type(owner) is not int or not 0 < owner < 2**64 or type(duration_ms) is not int or not 0 < duration_ms < 2**64:
            raise BuildError(400, "InvalidRequestWindow")
        rejected = None
        with self.transaction():
            now = self._advance()
            try:
                event = self._reduce({"Issue": {"tenant": owner, "now_ms": now, "duration_ms": duration_ms}})
            except WindowRejected as error:
                rejected = error
        if rejected:
            raise BuildError(429 if rejected.reason == "CapacityExceeded" else 409, "RequestWindow" + rejected.reason)
        record = event["Issued"]
        return {"requestWindow": {"namespace": self.bounded["namespace"], "sequence": record["id"]},
                "expiresAtMs": record["expires_at_ms"]}

    def close_window(self, owner, window):
        if not self.bounded:
            raise BuildError(409, "BuildWindowsDisabled")
        if type(owner) is not int or not 0 < owner < 2**64:
            raise BuildError(400, "InvalidOwner")
        self._window(window)
        rejected = None
        with self.transaction():
            now = self._advance()
            try:
                self._reduce({"Close": {"tenant": owner, "window": window["sequence"], "now_ms": now}})
            except WindowRejected as error:
                rejected = error
        if rejected:
            raise BuildError(409, "RequestWindow" + rejected.reason)
        return {"requestWindow": window, "closed": True}

    def _owned_files(self, row):
        directory = self.directory(row["id"])
        private_directory(directory)
        marker = directory / ".owner.json"
        expected = json.dumps({"id": row["id"], "nonce": row["fs_nonce"]}, sort_keys=True).encode()
        if marker.is_symlink() or marker.stat().st_size != len(expected) or marker.read_bytes() != expected:
            raise BuildError(503, "BuildWorkspaceIdentityUnknown")
        meta = directory.stat()
        if row["fs_identity"] and json.loads(row["fs_identity"]) != [meta.st_dev, meta.st_ino]:
            raise BuildError(503, "BuildWorkspaceIdentityUnknown")
        allowed = {".owner.json", "source.tar", "source.partial", "candidate.ext4", "candidate.partial",
                   "status.json", "status.partial", "build.log", "build.partial"}
        request = json.loads(row["request"])
        maximum = {".owner.json": len(expected), "source.tar": self.limits["max_input_bytes"],
                   "source.partial": self.limits["max_input_bytes"],
                   "candidate.ext4": request["resources"]["diskMiB"] * MIB,
                   "candidate.partial": request["resources"]["diskMiB"] * MIB,
                   "status.json": 128 * 1024, "status.partial": 128 * 1024,
                   "build.log": self.limits["max_log_bytes"], "build.partial": self.limits["max_log_bytes"]}
        files = []
        with os.scandir(directory) as entries:
            for item in entries:
                if item.name not in allowed or len(files) >= len(allowed):
                    raise BuildError(503, "BuildWorkspaceUnknownEntry")
                entry = item.stat(follow_symlinks=False)
                if (not stat.S_ISREG(entry.st_mode) or entry.st_nlink != 1 or entry.st_uid != os.geteuid()
                        or entry.st_size > maximum[item.name]):
                    raise BuildError(503, "BuildWorkspaceIdentityUnknown")
                files.append([item.name, entry.st_dev, entry.st_ino])
        return files

    def _prepare(self, job):
        row = self._row(job)
        if row["fs_phase"] != "Preparing":
            return
        directory = self.directory(job)
        pending = self.root / (".prepare-" + job)
        expected = json.dumps({"id": job, "nonce": row["fs_nonce"]}, sort_keys=True).encode()
        if directory.exists() or directory.is_symlink():
            # Final paths are never adopted, even when empty. They must carry
            # the committed reservation's complete identity marker.
            if pending.exists():
                raise BuildError(503, "BuildWorkspaceIdentityUnknown")
            self._owned_files(row)
        else:
            if not pending.exists():
                pending.mkdir(mode=0o700)
                self.fault("after_mkdir")
            private_directory(pending)
            # Preparing grants this one unpublished staging name under the
            # exclusive private-root lease. Empty/incomplete marker staging is
            # recoverable; unexpected names/types/content are preserved.
            with os.scandir(pending) as entries:
                names = []
                for item in entries:
                    if len(names) >= 2 or item.name not in {".owner.json", ".owner.partial"}:
                        raise BuildError(503, "BuildWorkspaceUnknownEntry")
                    meta = item.stat(follow_symlinks=False)
                    if (not stat.S_ISREG(meta.st_mode) or meta.st_nlink != 1 or meta.st_uid != os.geteuid()
                            or meta.st_size > len(expected) or meta.st_mode & 0o077):
                        raise BuildError(503, "BuildWorkspaceIdentityUnknown")
                    data = Path(item.path).read_bytes()
                    if (item.name == ".owner.json" and data != expected
                            or item.name == ".owner.partial" and not expected.startswith(data)):
                        raise BuildError(503, "BuildWorkspaceIdentityUnknown")
                    names.append(item.name)
            marker = pending / ".owner.json"
            partial = pending / ".owner.partial"
            if not marker.exists():
                with partial.open("wb") as output:
                    os.chmod(output.name, 0o600)
                    output.write(expected)
                    output.flush()
                    os.fsync(output.fileno())
                self.fault("after_marker_fsync")
                os.replace(partial, marker)
            else:
                partial.unlink(missing_ok=True)
            fsync_directory(pending)
            fsync_directory(self.root)
            self.fault("after_staging_fsync")
            os.rename(pending, directory)
            self.fault("after_prepare_rename")
        fsync_directory(directory)
        fsync_directory(self.root)
        self.fault("after_directory_fsync")
        meta = directory.stat()
        with self.transaction():
            self.db.execute("UPDATE jobs SET fs_phase='Ready',fs_identity=?,state=? WHERE id=? AND fs_phase='Preparing'",
                (json.dumps([meta.st_dev, meta.st_ino]), "AwaitingInput" if json.loads(row["request"])["kind"] == "dockerfile" else "Queued", job))

    def resume_filesystem(self):
        with self.lock:
            for row in self.rows():
                if row["fs_phase"] == "Preparing":
                    self._prepare(row["id"])
                elif row["fs_phase"] == "Deleting":
                    self._delete(row)

    def recover_filesystem(self):
        with self.lock:
            if self.bounded:
                known = {row[0] for row in self.db.execute("SELECT id FROM jobs")}
                allowed = known | {".prepare-" + row["id"] for row in self.rows() if row["fs_phase"] == "Preparing"} | {"lease", "builds.db", "builds.db-wal", "builds.db-shm"}
                with os.scandir(self.root) as entries:
                    for count, entry in enumerate(entries):
                        if count >= 2 * self.limits["max_jobs"] + 4 or entry.name not in allowed:
                            raise BuildError(503, "UnknownBuildWorkspaceEntry")
            for row in self.rows():
                if row["fs_phase"] == "Preparing":
                    self._prepare(row["id"])
                elif row["fs_phase"] == "Deleting":
                    self._delete(row)
                elif self.bounded and row["fs_phase"] == "Ready":
                    self._owned_files(row)

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

    def submit(self, owner, request, token=None, window=None):
        if type(owner) is not int or not 0 < owner < 2**64:
            raise BuildError(400, "InvalidOwner")
        request = self.validate(request)
        if self.bounded and not token:
            raise BuildError(400, "IdempotencyKeyRequired")
        token = token or secrets.token_hex(16)
        if not isinstance(token, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", token):
            raise BuildError(400, "InvalidIdempotencyKey")
        encoded = json.dumps(request, sort_keys=True)
        if self.bounded and len(encoded.encode()) > self.bounded["max_request_bytes"]:
            raise BuildError(413, "BuildRequestTooLarge")
        domain = self._window(window) if self.bounded and window is not None else ("", "")
        refused = None
        with self.lock:
            # Duplicate lookup is read-only and precedes WAL/free-space gates,
            # clock advancement and admission. A saturated journal must still
            # disclose its previously accepted original result.
            old = self.db.execute("SELECT * FROM jobs WHERE owner=? AND window_namespace=? AND window_sequence=? AND token=?",
                                  (str(owner), *domain, token)).fetchone()
            if old:
                if old["request"] != encoded:
                    raise BuildError(409, "IdempotencyConflict")
                return self.view(old)
            with self.transaction():
                if self.bounded:
                    now_ms = self._advance()
                    if window is None:
                        refused = BuildError(409, "LegacyBuildDomainClosed")
                    else:
                        try:
                            self._reduce({"Admit": {"tenant": owner, "window": window["sequence"], "now_ms": now_ms}})
                        except WindowRejected as error:
                            refused = BuildError(409, "RequestWindow" + error.reason)
                if refused is None:
                    counts = self.db.execute("SELECT COUNT(*), COALESCE(SUM(reserved),0) FROM jobs").fetchone()
                    own = self.db.execute("SELECT COUNT(*), COALESCE(SUM(reserved),0) FROM jobs WHERE owner=?", (str(owner),)).fetchone()
                    active = self.db.execute("SELECT COUNT(*) FROM jobs WHERE reserved>0").fetchone()[0]
                    own_active = self.db.execute("SELECT COUNT(*) FROM jobs WHERE owner=? AND reserved>0", (str(owner),)).fetchone()[0]
                    cost = (self.limits["builder_disk_bytes"] + self.limits["max_input_bytes"]
                            + 2 * request["resources"]["diskMiB"] * MIB + self.limits["artifact_margin_bytes"])
                    history = self.limits["max_log_bytes"] + (self.bounded["max_record_bytes"] if self.bounded else 256 * 1024)
                    journal = sum(self.bounded[k] for k in ("database_bytes", "wal_bytes", "recovery_reserve_bytes")) if self.bounded else 0
                    if self.physical_lease is not None:
                        try:
                            physical_ready = self.physical_lease.admit(self.free_bytes())
                        except (OSError, ValueError):
                            raise BuildError(503, "PhysicalStorageIdentityUnknown") from None
                    else:
                        physical_ready = True
                    if (not physical_ready or counts[0] >= self.limits["max_jobs"] or own[0] >= self.limits["max_jobs_per_tenant"]
                            or active >= self.limits["max_pending"] or own_active >= self.limits["max_pending_per_tenant"]
                            or counts[1] + cost + (counts[0] + 1) * history + journal > self.limits["workspace_bytes"]
                            or own[1] + cost + (own[0] + 1) * history > self.limits["workspace_bytes_per_tenant"]
                            or self.free_bytes() < counts[1] + cost + (counts[0] + 1) * history + journal + self.limits["runtime_cleanup_reserve_bytes"]):
                        refused = BuildError(429, "BuildBudgetExceeded")
                    else:
                        job = self._allocate_job() if self.bounded else secrets.token_hex(16)
                        if job is None:
                            refused = BuildError(429, "BuildIdentityExhausted")
                        else:
                            now = now_ms / 1000 if self.bounded else self.clock()
                            self.db.execute("""INSERT INTO jobs (id,owner,token,request,state,created,deadline,reserved,
                                window_namespace,window_sequence,fs_phase,fs_nonce) VALUES(?,?,?,?,?,?,?,?,?,?,'Preparing',?)""",
                                (job, str(owner), token, encoded, "Preparing", now, now + request["timeoutSeconds"], cost,
                                 *domain, secrets.token_hex(16)))
                            self._check_record(self._row(job))
            if refused:
                # AdvanceTime is already committed, including expiry refusal.
                raise refused
            self.fault("after_admission_commit")
            self._prepare(job)
            return self.view(self._row(job, owner))

    def _row(self, job, owner=None):
        identity(job)
        row = self.db.execute("SELECT * FROM jobs WHERE id=?", (job,)).fetchone()
        if row is None or (owner is not None and int(row["owner"]) != owner):
            raise BuildError(404, "NotFound")
        return dict(row) | {"owner": int(row["owner"])}

    def get(self, job, owner=None):
        with self.lock:
            return dict(self._row(job, owner))

    def view(self, row):
        return {"buildID": row["id"], "state": row["state"], "failure": row["failure"],
                "createdAt": row["created"], "deadline": row["deadline"],
                "template": json.loads(row["result"]) if row["result"] else None,
                "retainedUntilMs": row["retained_until_ms"]}

    def query(self, job, owner):
        with self.lock:
            return self.view(self._row(job, owner))

    def change(self, job, state, *, failure=None, candidate=None, result=None, release=False):
        with self.transaction():
            row = self._row(job)
            if row["state"] in TERMINAL:
                raise BuildError(409, "TerminalBuild")
            result_json = json.dumps(result) if result else None
            if self.bounded and result_json and len(result_json.encode()) > self.bounded["max_result_bytes"]:
                raise BuildError(413, "BuildResultTooLarge")
            until = row["retained_until_ms"]
            if self.bounded and state in TERMINAL:
                now = self._advance()
                until = now + self.bounded["retention_ms"]
                if until >= 2**63:
                    raise BuildError(503, "BuildRetentionOverflow")
            updated = row | {"state": state, "failure": failure, "candidate": json.dumps(candidate) if candidate else row["candidate"],
                             "result": result_json, "reserved": 0 if release else row["reserved"], "retained_until_ms": until}
            self._check_record(updated)
            self.db.execute("UPDATE jobs SET state=?,failure=?,candidate=?,result=?,reserved=?,retained_until_ms=? WHERE id=?",
                (state, failure, updated["candidate"], result_json, updated["reserved"], until, job))

    def private_checkpoint(self, job, **values):
        if not values or set(values) - {"private_window", "builder_settled", "private_ack"}:
            raise ValueError("invalid private settlement checkpoint")
        with self.transaction():
            row = self._row(job)
            update = dict(values)
            for name in ("private_window", "builder_settled"):
                if name in update:
                    update[name] = json.dumps(update[name], sort_keys=True)
                    if row[name] is not None and row[name] != update[name]:
                        raise BuildError(409, "PrivateBuildIdentityConflict")
            self._check_record(row | update)
            self.db.execute("UPDATE jobs SET " + ",".join(name + "=?" for name in update) + " WHERE id=?", (*update.values(), job))

    def staging_cleanup_checkpoint(self, job, manifest):
        with self.transaction():
            row = self._row(job)
            encoded = json.dumps(manifest, sort_keys=True) if manifest is not None else None
            if row["publication_attempted"]:
                raise BuildError(409, "PublicationOutcomeUnknown")
            if row["staging_cleanup"] and encoded and row["staging_cleanup"] != encoded:
                raise BuildError(409, "StagingCleanupIdentityConflict")
            self._check_record(row | {"staging_cleanup": encoded})
            self.db.execute("UPDATE jobs SET staging_cleanup=? WHERE id=?", (encoded, job))

    def retire_results(self):
        if not self.bounded:
            return
        with self.transaction():
            now = self._advance()
            opened = {str(record["id"]) for record in self._window_state()["open"]}
            for row in self.rows():
                # Legacy rows and unresolved work remain charged indefinitely.
                if (row["state"] not in TERMINAL or row["reserved"] or not row["window_namespace"]
                        or row["window_sequence"] in opened or row["retained_until_ms"] is None
                        or now < row["retained_until_ms"] or not row["private_ack"]
                        or not row["builder_settled"] or row["staging_cleanup"] or row["fs_phase"] != "Ready"):
                    continue
                manifest = self._owned_files(row)
                self._check_record(row | {"delete_manifest": json.dumps(manifest), "fs_phase": "Deleting"})
                self.db.execute("UPDATE jobs SET fs_phase='Deleting',delete_manifest=? WHERE id=?", (json.dumps(manifest), row["id"]))
        self.fault("after_delete_intent")
        with self.lock:
            for row in self.rows():
                if row["fs_phase"] == "Deleting":
                    self._delete(row)

    def _delete(self, row):
        directory = self.directory(row["id"])
        if directory.exists() or directory.is_symlink():
            private_directory(directory)
            meta = directory.stat()
            if json.loads(row["fs_identity"]) != [meta.st_dev, meta.st_ino]:
                raise BuildError(503, "BuildWorkspaceIdentityUnknown")
            expected = {name: (dev, ino) for name, dev, ino in json.loads(row["delete_manifest"])}
            with os.scandir(directory) as entries:
                actual = list(entries)
            if len(actual) > len(expected):
                raise BuildError(503, "BuildWorkspaceUnknownEntry")
            for entry in actual:
                meta = entry.stat(follow_symlinks=False)
                if entry.name not in expected or expected[entry.name] != (meta.st_dev, meta.st_ino):
                    raise BuildError(503, "BuildWorkspaceIdentityUnknown")
            for entry in actual:
                Path(entry.path).unlink()
                self.fault("after_delete_file")
            fsync_directory(directory)
            directory.rmdir()
        fsync_directory(self.root)
        self.fault("after_delete_fsync")
        with self.transaction():
            self.db.execute("DELETE FROM jobs WHERE id=? AND fs_phase='Deleting'", (row["id"],))

    def begin_publication(self, job):
        """Commit intent before POST; an unknown attempt is observation-only."""
        with self.transaction():
            row = self._row(job)
            if row["state"] != "Publishing" or row["publication_attempted"]:
                raise BuildError(409, "PublicationOutcomeUnknown")
            self.db.execute("UPDATE jobs SET publication_attempted=1 WHERE id=?", (job,))

    def clear_publication_attempt(self, job):
        """Only the original dispatch may prove it was not sent or was refused."""
        with self.transaction():
            row = self._row(job)
            if row["state"] != "Publishing":
                raise BuildError(409, "TerminalBuild")
            self.db.execute("UPDATE jobs SET publication_attempted=0 WHERE id=?", (job,))

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
            check = lambda: self.remaining_deadline(row["deadline"])
            path, published = self.directory(job) / "source.tar", []
            try:
                _, digest = durable_write(path, blocks, size, exact=size,
                                          check_deadline=check, on_publish=published.append)
                with self.transaction(check_before_commit=check):
                    check()
                    self.db.execute("UPDATE jobs SET state='Queued',input_hash=? WHERE id=? AND state='AwaitingInput'", (digest, job))
            except BaseException:
                # A lost COMMIT response may already have queued the original
                # input. Observe that row before removing an uncommitted file.
                with self.lock:
                    if published and self._row(job)["state"] == "AwaitingInput":
                        remove_owned_input(path, published[0])
                raise
        except BuildError as error:
            if error.reason in ("InputCleanupIdentityUnknown", "InputCleanupUnknown"):
                self.change(job, "CleanupUnknown", failure=error.reason)
            raise
        finally:
            with self.lock:
                self.uploads.discard(job)

    def remaining_deadline(self, deadline):
        """Keep the original job deadline across reads, durable I/O and admission."""
        remaining = deadline - self.clock()
        if remaining <= 0:
            raise BuildError(408, "BuildDeadlineExceeded")
        return remaining

    def rows(self):
        with self.lock:
            return [dict(row) | {"owner": int(row["owner"])} for row in self.db.execute("SELECT * FROM jobs ORDER BY created,id")]

    def logs(self, job, owner, offset, limit):
        with self.lock:
            self._row(job, owner)
            if type(offset) is not int or offset < 0 or type(limit) is not int or not 1 <= limit <= 65536:
                raise BuildError(400, "InvalidLogRange")
            path = self.directory(job) / "build.log"
            data = b""
            if path.exists():
                with path.open("rb") as incoming:
                    incoming.seek(min(offset, path.stat().st_size))
                    data = incoming.read(limit)
            return {"offset": offset, "nextOffset": offset + len(data), "text": data.decode("utf-8", errors="replace")}
