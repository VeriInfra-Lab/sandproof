"""Track exact fan-out effects before making any branch endpoint usable."""

from dataclasses import asdict, dataclass, field
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat

_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}")
_TOKEN = re.compile(r"[A-Za-z0-9_-]{16,128}")


def identifier(value):
    if not isinstance(value, str) or not _ID.fullmatch(value):
        raise ValueError("invalid fan-out identity")
    return value


@dataclass(frozen=True)
class BranchRequest:
    branch_id: str
    port: int
    access_token: str = field(repr=False)


@dataclass(frozen=True)
class Plan:
    batch_id: str
    checkpoint_id: str
    branches: tuple[BranchRequest, ...]

    @classmethod
    def parse(cls, value, max_branches, ports):
        if type(value) is not dict or value.keys() != {"batch_id", "checkpoint_id", "branches"}:
            raise ValueError("invalid fan-out plan")
        batch_id = identifier(value["batch_id"])
        checkpoint_id = identifier(value["checkpoint_id"])
        raw = value["branches"]
        if type(raw) is not list or not 1 <= len(raw) <= max_branches:
            raise ValueError("invalid branch count")
        branches = []
        for item in raw:
            if type(item) is not dict or item.keys() != {"branch_id", "port", "access_token"}:
                raise ValueError("invalid branch request")
            branch_id = identifier(item["branch_id"])
            port = item["port"]
            if type(port) is not int or port not in ports:
                raise ValueError("port outside reserved fan-out range")
            token = item["access_token"]
            if not isinstance(token, str) or not _TOKEN.fullmatch(token):
                raise ValueError("invalid envd access token")
            branches.append(BranchRequest(branch_id, port, token))
        for name in ("branch_id", "port", "access_token"):
            if len({getattr(item, name) for item in branches}) != len(branches):
                raise ValueError("branch identities, ports and tokens must be distinct")
        return cls(batch_id, checkpoint_id, tuple(branches))


def private_directory(path):
    path.mkdir(mode=0o700, exist_ok=True)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o077:
        raise ValueError("fan-out directory must be private to its owner")


def sync_directory(path):
    fd = os.open(path, os.O_DIRECTORY | os.O_RDONLY | os.O_CLOEXEC)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


class GuestFanout:
    """Serialized exact-batch journal scoped to one runtime incarnation.

    The backend creates each child inside its already-reserved ordinal. Its cleanup must prove empty and must fence
    any late start for that ordinal before returning.
    """

    def __init__(self, state, checkpoint_id, backend, *, max_branches, ports, max_batches):
        self.state = Path(state)
        self.checkpoint_id = identifier(checkpoint_id)
        if (type(max_branches) is not int or not 1 <= max_branches <= 64
                or type(max_batches) is not int or not 1 <= max_batches <= 4096
                or not isinstance(ports, range) or ports.step != 1
                or not 1024 <= ports.start < ports.stop <= 65536
                or len(ports) < max_branches):
            raise ValueError("invalid fan-out limits")
        self.backend = backend
        self.max_branches = max_branches
        self.max_batches = max_batches
        self.ports = ports
        private_directory(self.state)
        self._lock = os.open(self.state / "owner.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        try:
            fcntl.flock(self._lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BaseException:
            self.close()
            raise

    def close(self):
        if self._lock is not None:
            os.close(self._lock)
            self._lock = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def _load(self, batch_id):
        if self._lock is None:
            raise RuntimeError("fan-out ownership has ended")
        try:
            return json.loads((self.state / (identifier(batch_id) + ".json")).read_bytes())
        except FileNotFoundError:
            return None

    def _save(self, row):
        name = row["batch_id"] + ".json"
        stage = self.state / (name + ".new")
        # A failed prior publication can leave a stage file. The committed row,
        # if any, remains the authority; no external effect starts from a stage.
        stage.unlink(missing_ok=True)
        fd = os.open(stage, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            json.dump(row, output, separators=(",", ":"))
            output.flush()
            os.fsync(output.fileno())
        stage.replace(self.state / name)
        sync_directory(self.state)

    @staticmethod
    def _public(row, status, branches=()):
        result = {"batch_id": row["batch_id"], "status": status}
        if status == "ready":
            result.update(branches=list(branches), released=list(row["released"]))
        return result

    def _occupied(self):
        branch_ids, ports, count = set(), set(), 0
        for file in self.state.glob("*.json"):
            count += 1
            row = json.loads(file.read_bytes())
            for branch in row["branches"]:
                if row["phase"] in ("failed_clean", "released") or branch["branch_id"] in row["released"]:
                    continue
                branch_ids.add(branch["branch_id"])
                ports.add(branch["port"])
        return branch_ids, ports, count

    def start(self, value):
        plan = Plan.parse(value, self.max_branches, self.ports)
        identity = hashlib.sha256(json.dumps(asdict(plan), sort_keys=True).encode()).hexdigest()
        row = self._load(plan.batch_id)
        if row is not None:
            if row["identity"] != identity:
                raise ValueError("batch identity already binds another request")
            return self._observe(row)
        if plan.checkpoint_id != self.checkpoint_id:
            raise ValueError("checkpoint identity is not active in this runtime")
        branch_ids, ports, count = self._occupied()
        if count >= self.max_batches:
            return self._absent_with_full_history(plan.batch_id, count, unknown="pending")
        requested = {branch.branch_id for branch in plan.branches}
        if (len(branch_ids) + len(requested) > self.max_branches or branch_ids & requested
                or ports & {branch.port for branch in plan.branches}):
            raise ValueError("fan-out capacity or endpoint already reserved")
        row = {
            "batch_id": plan.batch_id,
            "identity": identity,
            "phase": "starting",
            "branches": [{"branch_id": branch.branch_id, "port": branch.port} for branch in plan.branches],
            "released": [],
            "retiring": [],
        }
        self._save(row)
        try:
            self.backend.start(plan)
        except (OSError, ValueError, RuntimeError):
            # Failed or missing donor reply is compatible with a partially
            # started batch. Retain the lease until exact cleanup succeeds.
            return self._retire(row, range(len(plan.branches)), "failed_clean")
        row["phase"] = "ready"
        self._save(row)
        return self._observe(row)

    def probe(self):
        if self._lock is None or not self.backend.checkpoint_available():
            return {"status": "unavailable"}
        return {"status": "checkpoint_ready", "checkpoint_id": self.checkpoint_id,
                "max_branches": self.max_branches}

    def observe(self, batch_id):
        row = self._load(batch_id)
        if row is None:
            _, _, count = self._occupied()
            return self._absent_with_full_history(batch_id, count, unknown="lost")
        return self._observe(row)

    def _observe(self, row):
        phase = row["phase"]
        if phase == "starting":
            return self._retire(row, range(len(row["branches"])), "failed_clean")
        if phase == "failing":
            return self._retire(row, row["retiring"], "failed_clean")
        if phase != "ready":
            return self._public(row, phase)
        if row["retiring"]:
            indexes = row["retiring"]
            phase = "released" if len(row["released"]) + len(indexes) == len(row["branches"]) else "ready"
            return self._retire(row, indexes, phase)
        active = [(i, branch) for i, branch in enumerate(row["branches"])
                  if branch["branch_id"] not in row["released"]]
        uncertain = False
        for index, branch in active:
            try:
                present = self.backend.present(row["batch_id"], index, branch["port"])
            except OSError:
                uncertain = True
                continue
            if not present:
                return self._retire(row, (ordinal for ordinal, _ in active), "failed_clean")
        if uncertain:
            return self._public(row, "pending")
        return self._public(row, "ready", [branch for _, branch in active])

    def release(self, batch_id, branch_id=None):
        row = self._load(batch_id)
        if row is None:
            if branch_id is not None:
                return {"batch_id": batch_id, "status": "lost"}
            return self._record_closed_before_start(batch_id)
        if branch_id is not None and identifier(branch_id) not in {b["branch_id"] for b in row["branches"]}:
            raise ValueError("branch identity does not belong to this batch")
        if row["phase"] != "ready":
            return self._observe(row)
        remaining = [i for i, item in enumerate(row["branches"]) if item["branch_id"] not in row["released"]]
        indexes = set(row["retiring"])
        indexes.update(i for i in remaining if branch_id in (None, row["branches"][i]["branch_id"]))
        if not indexes:
            return self._observe(row)
        return self._retire(row, sorted(indexes), "released" if len(indexes) == len(remaining) else "ready")

    def _scope_absent(self, batch_id):
        try:
            return not self.backend.has_batch(batch_id)
        except OSError:
            return False

    def _absent_with_full_history(self, batch_id, count, *, unknown):
        # A successful Start needs a new durable row before any effect. Once
        # this lifetime budget fills, no future request can add the absent row.
        status = "never_started" if count == self.max_batches and self._scope_absent(batch_id) else unknown
        return {"batch_id": batch_id, "status": status}

    def _record_closed_before_start(self, batch_id):
        # Dispatch occurs only after committing the original plan, and both
        # requests run under this same live journal owner. Fence a later Start
        # before reporting absence as cleanup.
        _, _, count = self._occupied()
        if count > self.max_batches or not self._scope_absent(batch_id):
            return {"batch_id": batch_id, "status": "pending"}
        if count == self.max_batches:
            return {"batch_id": batch_id, "status": "released"}
        row = {"batch_id": batch_id, "identity": None, "phase": "released",
               "branches": [], "released": [], "retiring": []}
        self._save(row)
        return self._public(row, "released")

    def _retire(self, row, indexes, terminal):
        row["retiring"] = list(indexes)
        if terminal == "failed_clean":
            # Recovery must distinguish a failed batch from ordinary release,
            # including when some members were individually released earlier.
            row["phase"] = "failing"
        self._save(row)
        try:
            for ordinal in row["retiring"]:
                self.backend.release(row["batch_id"], ordinal)
        except (OSError, RuntimeError):
            return self._public(row, "pending")
        if terminal == "failed_clean" and not self._scope_absent(row["batch_id"]):
            return self._public(row, "pending")
        row["released"].extend(row["branches"][i]["branch_id"] for i in row["retiring"]
                               if row["branches"][i]["branch_id"] not in row["released"])
        row["retiring"] = []
        row["phase"] = terminal
        self._save(row)
        return self._observe(row)
