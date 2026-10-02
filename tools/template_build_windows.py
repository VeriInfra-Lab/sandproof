"""Bounded bridge to the existing pure Rust request-window reducer.

BuildStore supplies persisted state and authenticated input under its existing
RLock + BEGIN IMMEDIATE. Only
that caller may persist a returned decision, atomically with its job ledger.
No database, business clock, idempotency ledger, commit, or dispatch lives here.
Monotonic time is used only to bound the lifetime of the owned helper process.
"""
from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import re
import selectors
import stat
import subprocess
import sys
import time


VERSION = 1
ERROR_CODES = frozenset({"invalid_request", "unsupported_version", "input_too_large",
                         "output_too_large", "input_io"})
REJECTIONS = frozenset({"InvalidLimits", "InvalidState", "InvalidDuration", "ClockRegression",
                        "TimeOverflow", "IdExhausted", "CapacityExceeded", "UnknownWindow",
                        "ClosedWindow", "WrongTenant"})
# Protocol framing minimum, not a deployment budget/default. Includes no newline.
MIN_OUTPUT_BYTES = max(len(json.dumps({"version": VERSION, "status": "error", "code": code},
                                     separators=(",", ":")).encode()) for code in ERROR_CODES)
U64_MAX = 2**64 - 1


class WindowBridgeError(RuntimeError):
    """A stable local failure code; no state or untrusted helper text is exposed."""
    def __init__(self, code):
        self.code = code
        super().__init__(code)


class WindowRejected(WindowBridgeError):
    """The pure reducer refused the input. The caller still owns unchanged state."""
    def __init__(self, reason):
        self.reason = reason
        super().__init__("rejected:" + reason)


@dataclass(frozen=True)
class WindowHelper:
    # Explicit identity and limits. Production root services use owner_uid=0;
    # tests explicitly authorize their own UID and exact compiled ELF digest.
    path: Path
    sha256: str
    owner_uid: int
    max_input_bytes: int
    max_output_bytes: int
    timeout_seconds: float

    def __post_init__(self):
        try:
            path = Path(self.path)
            finite_timeout = math.isfinite(self.timeout_seconds)
        except (TypeError, ValueError, OverflowError) as error:
            raise WindowBridgeError("invalid_config") from error
        if (not path.is_absolute() or ".." in path.parts or str(path) != str(self.path)
                or type(self.sha256) is not str or not re.fullmatch("[0-9a-f]{64}", self.sha256)
                or type(self.owner_uid) is not int or not 0 <= self.owner_uid < 2**32
                or type(self.max_input_bytes) is not int
                or not 0 < self.max_input_bytes < sys.maxsize
                or type(self.max_output_bytes) is not int
                or not MIN_OUTPUT_BYTES <= self.max_output_bytes < sys.maxsize
                or type(self.timeout_seconds) not in (int, float)
                or not finite_timeout or self.timeout_seconds <= 0):
            raise WindowBridgeError("invalid_config")
        object.__setattr__(self, "path", path)


def _object(value, fields):
    if type(value) is not dict or value.keys() != fields:
        raise ValueError("object shape")


def _u64(value):
    if type(value) is not int or not 0 <= value <= U64_MAX:
        raise ValueError("u64")


def _record(record):
    _object(record, {"id", "tenant", "issued_at_ms", "expires_at_ms"})
    for value in record.values():
        _u64(value)


def _state(state):
    """Wire shape only; the Rust core alone validates/reduces state semantics."""
    _object(state, {"limits", "issued_through", "time_floor_ms", "open"})
    _object(state["limits"], {"max_open_windows", "max_duration_ms"})
    for value in state["limits"].values():
        _u64(value)
    if state["issued_through"] is not None:
        _u64(state["issued_through"])
    _u64(state["time_floor_ms"])
    if type(state["open"]) is not list:
        raise ValueError("window records")
    for record in state["open"]:
        _record(record)


def _input(value):
    if type(value) is not dict or len(value) != 1:
        raise ValueError("input variant")
    name, fields = next(iter(value.items()))
    shapes = {"Issue": {"tenant", "now_ms", "duration_ms"},
              "Close": {"tenant", "window", "now_ms"},
              "AdvanceTime": {"now_ms"}, "Admit": {"tenant", "window", "now_ms"}}
    if name not in shapes:
        raise ValueError("input variant")
    _object(fields, shapes[name])
    for field in fields.values():
        _u64(field)


def _event(value):
    if type(value) is not dict or len(value) != 1:
        raise ValueError("event variant")
    name, fields = next(iter(value.items()))
    if name in {"Issued", "AdmissionValidated"}:
        _record(fields)
    elif name in {"Closed", "TimeAdvanced"}:
        _object(fields, {"window"} if name == "Closed" else {"now_ms", "expired"})
        for field in fields.values():
            _u64(field)
    else:
        raise ValueError("event variant")


def _request(state, window_input, maximum):
    try:
        # Every valid record occupies more than one byte. Bound traversal before
        # visiting records; subsequent encoding never creates oversized tokens.
        if (type(state) is dict and type(state.get("open")) is list
                and len(state["open"]) > maximum):
            raise WindowBridgeError("input_too_large")
        _state(state)
        _input(window_input)
        request = {"version": VERSION, "state": state, "input": window_input}
        result = bytearray()
        for part in json.JSONEncoder(separators=(",", ":"), allow_nan=False).iterencode(request):
            raw = part.encode("utf-8")
            if len(raw) > maximum - len(result):
                raise WindowBridgeError("input_too_large")
            result.extend(raw)
        return bytes(result)
    except (ValueError, TypeError, RecursionError, OverflowError) as error:
        raise WindowBridgeError("invalid_request") from error


def _identity(info):
    return (info.st_dev, info.st_ino, info.st_mode, info.st_uid, info.st_gid,
            info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def _open_helper(spec):
    """Walk protected directories by FD, then hash and exec the same open inode.

    Root and the explicitly configured owner are trusted; neither group/other
    writability nor symlink components are allowed, including the path root.
    Ownership is a deployment trust boundary, not protection against that owner.
    """
    directory = None
    executable = None
    try:
        directory = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        for part in (*spec.path.parts[1:-1], None):
            info = os.fstat(directory)
            if info.st_uid not in {0, spec.owner_uid} or info.st_mode & 0o022:
                raise WindowBridgeError("helper_identity")
            if part is not None:
                child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                                dir_fd=directory)
                os.close(directory)
                directory = child
        executable = os.open(spec.path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                             dir_fd=directory)
        before = os.fstat(executable)
        if (not stat.S_ISREG(before.st_mode) or before.st_uid != spec.owner_uid
                or before.st_mode & 0o022 or not before.st_mode & 0o111):
            raise WindowBridgeError("helper_identity")
        digest = hashlib.sha256()
        header = os.read(executable, 4)
        if header != b"\x7fELF":
            # No shebang interpreter or secondary executable identity to trust.
            raise WindowBridgeError("helper_identity")
        digest.update(header)
        while chunk := os.read(executable, 65536):
            digest.update(chunk)
        if digest.hexdigest() != spec.sha256 or _identity(os.fstat(executable)) != _identity(before):
            raise WindowBridgeError("helper_identity")
        result = executable
        executable = None
        return result
    except OSError as error:
        raise WindowBridgeError("helper_identity") from error
    finally:
        if executable is not None:
            os.close(executable)
        if directory is not None:
            os.close(directory)


def _exchange(child, request, maximum, deadline):
    """Drain both pipes as bytes arrive, never communicate() into unbounded RAM."""
    output = bytearray()
    received = 0
    sent = 0
    with selectors.DefaultSelector() as ready:
        for stream, events, name in ((child.stdin, selectors.EVENT_WRITE, "input"),
                                     (child.stdout, selectors.EVENT_READ, "output"),
                                     (child.stderr, selectors.EVENT_READ, "error")):
            os.set_blocking(stream.fileno(), False)
            ready.register(stream, events, name)
        while ready.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise WindowBridgeError("helper_timeout")
            for key, _ in ready.select(min(remaining, 1.0)):
                if key.data == "input":
                    try:
                        sent += os.write(key.fd, request[sent:sent + 65536])
                    except BrokenPipeError as error:
                        raise WindowBridgeError("helper_failed") from error
                    if sent == len(request):
                        ready.unregister(key.fileobj)
                        key.fileobj.close()
                else:
                    data = os.read(key.fd, min(65536, maximum - received + 1))
                    if not data:
                        ready.unregister(key.fileobj)
                        key.fileobj.close()
                    else:
                        if key.data == "error":
                            # Any stderr byte violates this wire protocol.
                            raise WindowBridgeError("helper_stderr")
                        received += len(data)
                        if received > maximum:
                            raise WindowBridgeError("output_too_large")
                        output.extend(data)
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise WindowBridgeError("helper_timeout")
        try:
            if child.wait(timeout=remaining) != 0:
                raise WindowBridgeError("helper_failed")
        except subprocess.TimeoutExpired as error:
            raise WindowBridgeError("helper_timeout") from error
    return bytes(output)


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate field")
        result[key] = value
    return result


def _nonfinite(_value):
    raise ValueError("nonfinite number")


def _response(raw):
    try:
        response = json.loads(raw.decode("utf-8"), object_pairs_hook=_pairs,
                              parse_constant=_nonfinite)
        if (type(response) is not dict or type(response.get("version")) is not int
                or response["version"] != VERSION):
            raise ValueError("response version")
        status = response.get("status")
        if status == "ok":
            _object(response, {"version", "status", "decision"})
            decision = response["decision"]
            _object(decision, {"event", "next"})
            _event(decision["event"])
            _state(decision["next"])
            return decision
        if status == "rejected":
            _object(response, {"version", "status", "reason"})
            if response["reason"] not in REJECTIONS:
                raise ValueError("rejection")
            raise WindowRejected(response["reason"])
        if status == "error":
            _object(response, {"version", "status", "code"})
            if response["code"] not in ERROR_CODES:
                raise ValueError("error code")
            raise WindowBridgeError(response["code"])
        raise ValueError("response status")
    except (ValueError, TypeError, UnicodeError, RecursionError, OverflowError) as error:
        raise WindowBridgeError("invalid_response") from error


class WindowReducer:
    def __init__(self, helper):
        if type(helper) is not WindowHelper:
            raise WindowBridgeError("invalid_config")
        self.helper = helper

    def reduce(self, state, window_input):
        """Return {event, next}, or refuse without changing the caller's state.

        The owner must first consult the accepted-job ledger. Admit is only
        validation: repeated calls are not deduplicated and reserve no work.
        Before reporting expiry, commit AdvanceTime separately as required by
        the core contract. This adapter never commits either decision.
        """
        spec = self.helper
        request = _request(state, window_input, spec.max_input_bytes)
        executable = _open_helper(spec)
        child = None
        try:
            # /proc/self/fd pins the verified inode across rename/replacement.
            # A minimal environment prevents LD_PRELOAD/PYTHONPATH injection.
            deadline = time.monotonic() + spec.timeout_seconds
            child = subprocess.Popen(
                [str(spec.path), "--max-input-bytes", str(spec.max_input_bytes),
                 "--max-output-bytes", str(spec.max_output_bytes)],
                executable=f"/proc/self/fd/{executable}", pass_fds=(executable,),
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                env={"LC_ALL": "C"}, close_fds=True, bufsize=0,
            )
            raw = _exchange(child, request, spec.max_output_bytes, deadline)
            return _response(raw)
        except OSError as error:
            raise WindowBridgeError("helper_failed") from error
        finally:
            os.close(executable)
            if child is not None:
                # The unreaped owned child cannot be PID-reused. Never kill by
                # executable name, process group, directory, or another caller's PID.
                if child.poll() is None:
                    child.kill()
                child.wait()
                for stream in (child.stdin, child.stdout, child.stderr):
                    stream.close()
