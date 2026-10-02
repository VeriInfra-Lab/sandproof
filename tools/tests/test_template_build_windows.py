"""Compiled-core bridge regression, with explicitly pinned compiled fault helpers.

All capacities/durations below are synthetic fixtures, not deployment defaults.
No BuildStore/job ledger integration or idempotency claim is made by this suite.
"""
import copy
from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
from types import SimpleNamespace
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools.template_build_windows import (
    MIN_OUTPUT_BYTES, WindowBridgeError, WindowHelper, WindowReducer, WindowRejected,
)

ROOT = Path(__file__).resolve().parents[2]
FAULT_SOURCE = r'''
use std::io::{Read, Write};
fn main() {
    let name = std::env::args().next().unwrap();
    let mode = std::path::Path::new(&name).file_name().unwrap().to_str().unwrap();
    if mode == "timeout" || mode == "blocked-input" {
        loop { std::thread::park(); }
    }
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
    if mode == "stdout-flood" || mode == "stderr-flood" {
        let mut pipe: Box<dyn Write> = if mode == "stdout-flood" {
            Box::new(std::io::stdout())
        } else { Box::new(std::io::stderr()) };
        loop { if pipe.write_all(&[b'x'; 4096]).is_err() { break; } }
        return;
    }
    let bytes: &[u8] = match mode {
        "truncated" => br#"{"version":1,"status":"ok""#,
        "duplicate" => br#"{"version":1,"version":1,"status":"rejected","reason":"ClosedWindow"}"#,
        "unknown-field" => br#"{"version":1,"status":"rejected","reason":"ClosedWindow","next":{}}"#,
        "wrong-reason" => br#"{"version":1,"status":"rejected","reason":"Whatever"}"#,
        "wrong-version" => br#"{"version":2,"status":"rejected","reason":"ClosedWindow"}"#,
        "bool-version" => br#"{"version":true,"status":"rejected","reason":"ClosedWindow"}"#,
        "bad-decision" => br#"{"version":1,"status":"ok","decision":{"event":{},"next":{}}}"#,
        "nonfinite" => br#"{"version":NaN,"status":"error","code":"invalid_request"}"#,
        "extra-json" => br#"{"version":1,"status":"rejected","reason":"ClosedWindow"}{}"#,
        "non-utf8" => &[0xff],
        "exit" => std::process::exit(19),
        _ => b"garbage",
    };
    std::io::stdout().write_all(bytes).unwrap();
}
'''


def fresh(count=2, duration=10):
    return {"limits": {"max_open_windows": count, "max_duration_ms": duration},
            "issued_through": None, "time_floor_ms": 0, "open": []}


def issue(now=1, duration=5, tenant=7):
    return {"Issue": {"tenant": tenant, "now_ms": now, "duration_ms": duration}}


def admit(now=1, tenant=7, window=0):
    return {"Admit": {"tenant": tenant, "window": window, "now_ms": now}}


def wire(state=None, window_input=None):
    return {"version": 1, "state": fresh() if state is None else state,
            "input": issue() if window_input is None else window_input}


def encoded(value):
    return json.dumps(value, separators=(",", ":")).encode()


class WindowBridgeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Required real compilation: missing toolchain is failure, never a skip.
        subprocess.run(["cargo", "build", "--locked", "-p", "sandproof-controller-service",
                        "--bin", "sandproof-window-reducer"], cwd=ROOT, check=True, timeout=180)
        target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
        if not target.is_absolute():
            target = ROOT / target
        cls.binary = target / "debug/sandproof-window-reducer"
        cls.digest = hashlib.sha256(cls.binary.read_bytes()).hexdigest()
        # Path components are tested as-is; no permission/owner relaxation.
        cls.temp = tempfile.TemporaryDirectory(prefix="window-bridge-", dir=target)
        cls.addClassCleanup(cls.temp.cleanup)
        cls.directory = Path(cls.temp.name)
        source = cls.directory / "fault.rs"
        source.write_text(FAULT_SOURCE)
        cls.fault = cls.directory / "fault"
        subprocess.run(["rustc", "--edition=2024", str(source), "-o", str(cls.fault)],
                       check=True, timeout=60)
        cls.fault_digest = hashlib.sha256(cls.fault.read_bytes()).hexdigest()

    def spec(self, **kwargs):
        return replace(WindowHelper(self.binary, self.digest, os.geteuid(), 4096, 4096, 5),
                       **kwargs)

    def reducer(self, **kwargs):
        return WindowReducer(self.spec(**kwargs))

    def fault_spec(self, mode, **kwargs):
        path = self.directory / mode
        if not path.exists():
            shutil.copyfile(self.fault, path)
            # Install mode at creation via copy2 rather than relaxing a live path.
            shutil.copymode(self.fault, path)
        return self.spec(path=path, sha256=self.fault_digest, **kwargs)

    def raw(self, data, input_limit=4096, output_limit=4096):
        if type(data) is dict:
            data = encoded(data)
        completed = subprocess.run([str(self.binary), "--max-input-bytes", str(input_limit),
                                    "--max-output-bytes", str(output_limit)],
                                   input=data, capture_output=True, timeout=5, check=True)
        self.assertEqual(completed.stderr, b"")
        self.assertLessEqual(len(completed.stdout), output_limit)
        return json.loads(completed.stdout)

    def assert_code(self, code, call):
        with self.assertRaises(WindowBridgeError) as error:
            call()
        self.assertEqual(error.exception.code, code)

    def assert_rejected(self, reason, state, window_input):
        before = copy.deepcopy(state)
        with self.assertRaises(WindowRejected) as error:
            self.reducer().reduce(state, window_input)
        self.assertEqual(error.exception.reason, reason)
        self.assertEqual(state, before)

    def test_exact_successor_and_event_from_real_compiled_core(self):
        state = fresh()
        before = copy.deepcopy(state)
        result = self.reducer().reduce(state, issue())
        record = {"id": 0, "tenant": 7, "issued_at_ms": 1, "expires_at_ms": 6}
        expected = {"event": {"Issued": record}, "next": {
            "limits": state["limits"], "issued_through": 0,
            "time_floor_ms": 1, "open": [record]}}
        self.assertEqual(result, expected)
        self.assertEqual(self.raw(wire())["decision"], expected)
        self.assertEqual(state, before)
        # No shared mutable references between decoded successor and old state.
        result["next"]["limits"]["max_open_windows"] = 99
        self.assertEqual(state, before)

    def test_repeated_admit_validates_same_window_without_job_deduplication(self):
        state = self.reducer().reduce(fresh(), issue())["next"]
        first = self.reducer().reduce(state, admit(2))
        repeated = self.reducer().reduce(first["next"], admit(2))
        self.assertEqual(first, repeated)
        self.assertEqual(first["event"], {"AdmissionValidated": state["open"][0]})
        closed = self.reducer().reduce(first["next"], {
            "Close": {"tenant": 7, "window": 0, "now_ms": 2}})
        self.assert_rejected("ClosedWindow", closed["next"], admit(2))
        self.assert_rejected("ClosedWindow", closed["next"], {
            "Close": {"tenant": 7, "window": 0, "now_ms": 2}})
        new = self.reducer().reduce(closed["next"], issue(2))
        self.assertEqual(new["event"]["Issued"]["id"], 1)

    def test_expiry_rejection_does_not_advance_clock_and_explicit_successor_does(self):
        state = self.reducer().reduce(fresh(), issue())["next"]
        self.assert_rejected("ClosedWindow", state, admit(6))
        self.assertEqual(state["time_floor_ms"], 1)
        advanced = self.reducer().reduce(state, {"AdvanceTime": {"now_ms": 6}})
        self.assertEqual(advanced["event"], {"TimeAdvanced": {"now_ms": 6, "expired": 1}})
        self.assertEqual(advanced["next"]["open"], [])
        self.assertEqual(advanced["next"]["issued_through"], 0)
        self.assert_rejected("ClockRegression", advanced["next"], admit(5))
        self.assert_rejected("ClosedWindow", advanced["next"], admit(6))
        self.assert_rejected("UnknownWindow", advanced["next"], admit(6, window=1))

    def test_reducer_refusals_and_integer_boundaries(self):
        self.assert_rejected("InvalidDuration", fresh(), issue(duration=0))
        self.assert_rejected("InvalidDuration", fresh(), issue(duration=11))
        self.assert_rejected("TimeOverflow", fresh(), issue(now=2**64 - 1))
        state = self.reducer().reduce(fresh(count=1), issue())["next"]
        self.assert_rejected("CapacityExceeded", state, issue(2))
        self.assert_rejected("WrongTenant", state, admit(2, tenant=8))
        self.assert_rejected("ClockRegression", state, admit(0))
        state = fresh()
        state["issued_through"] = 2**64 - 1
        self.assert_rejected("IdExhausted", state, issue())

    def test_actual_helper_rejects_unknown_and_duplicate_fields_at_every_layer(self):
        request = wire()
        variants = []
        for target in [(), ("state",), ("state", "limits"), ("input", "Issue")]:
            candidate = copy.deepcopy(request)
            current = candidate
            for key in target:
                current = current[key]
            current["unknown"] = 0
            variants.append(encoded(candidate))
        issued = self.reducer().reduce(fresh(), issue())["next"]
        candidate = wire(issued, admit())
        candidate["state"]["open"][0]["unknown"] = 0
        variants.append(encoded(candidate))
        for key, value in [("version", 1), ("time_floor_ms", 0), ("max_duration_ms", 10),
                           ("tenant", 7)]:
            original = encoded(request)
            field = encoded(key) + b":" + encoded(value)
            variants.append(original.replace(field, field + b"," + field, 1))
        record_wire = encoded(wire(issued, admit()))
        variants.append(record_wire.replace(b'"id":0', b'"id":0,"id":0', 1))
        for data in variants:
            with self.subTest(data=data):
                self.assertEqual(self.raw(data)["code"], "invalid_request")

    def test_actual_helper_rejects_malformed_truncated_extra_and_invalid_state(self):
        for data in [b"", b"{", encoded(wire())[:-1], encoded(wire()) + b"{}", b"null",
                     b"[]", b"\xff", b"{\"version\":NaN}"]:
            self.assertEqual(self.raw(data)["code"], "invalid_request")
        for state in [dict(fresh(), limits={"max_open_windows": 0, "max_duration_ms": 10}),
                      dict(fresh(), time_floor_ms=-1), dict(fresh(), issued_through=True),
                      dict(fresh(), open=[{"id": 0, "tenant": 1, "issued_at_ms": 0,
                                           "expires_at_ms": 1}])]:
            self.assertEqual(self.raw(wire(state))["code"], "invalid_request")
        for invalid in [True, -1, 2**64, "1", 1.0]:
            request = wire(window_input=issue(now=invalid))
            self.assertEqual(self.raw(request)["code"], "invalid_request")
        request = wire()
        request["version"] = 2
        self.assertEqual(self.raw(request)["code"], "unsupported_version")

    def test_input_boundary_and_output_bound_are_enforced_by_both_sides(self):
        data = encoded(wire())
        self.assertEqual(self.raw(data, input_limit=len(data))["status"], "ok")
        self.assertEqual(self.raw(data, input_limit=len(data)-1)["code"], "input_too_large")
        self.assertEqual(self.raw(wire(), output_limit=MIN_OUTPUT_BYTES)["code"], "output_too_large")
        self.assert_code("input_too_large", lambda: self.reducer(max_input_bytes=len(data)-1)
                         .reduce(fresh(), issue()))
        self.assert_code("output_too_large", lambda: self.reducer(max_output_bytes=MIN_OUTPUT_BYTES)
                         .reduce(fresh(), issue()))
        for limit in range(MIN_OUTPUT_BYTES, 350):
            response = self.raw(data, output_limit=limit)
            self.assertIn(response["status"], {"ok", "error"})
            if response["status"] == "error":
                self.assertEqual(response["code"], "output_too_large")
                self.assertNotIn("decision", response)

    def test_local_invalid_input_does_not_spawn_or_mutate(self):
        for state, value in [(fresh(), issue(now=True)), (fresh(), {"Issue": {"extra": 1}}),
                             (dict(fresh(), open="x"), issue())]:
            before = copy.deepcopy(state)
            with patch("tools.template_build_windows.subprocess.Popen") as spawn:
                self.assert_code("invalid_request", lambda: self.reducer().reduce(state, value))
                spawn.assert_not_called()
            self.assertEqual(state, before)

    def test_explicit_config_has_no_defaults_and_rejects_invalid_limits(self):
        for changes in [{"max_input_bytes": 0}, {"max_input_bytes": True},
                        {"max_output_bytes": MIN_OUTPUT_BYTES - 1},
                        {"timeout_seconds": 0}, {"timeout_seconds": float("inf")},
                        {"timeout_seconds": float("nan")}, {"timeout_seconds": True},
                        {"timeout_seconds": 10**1000}, {"timeout_seconds": "1"},
                        {"path": None},
                        {"owner_uid": -1}, {"owner_uid": True}, {"sha256": "0"},
                        {"path": Path("relative")}, {"path": Path("/tmp/../helper")}]:
            with self.subTest(changes=changes):
                self.assert_code("invalid_config", lambda: self.spec(**changes))
        with self.assertRaises(TypeError):
            WindowHelper(self.binary, self.digest, os.geteuid())

    def test_pinned_identity_rejects_digest_owner_symlink_directory_and_script(self):
        link = self.directory / "symlink"
        link.symlink_to(self.binary)
        script = self.directory / "script"
        with open(script, "w") as stream:
            stream.write("#!/bin/sh\nexit 0\n")
        cases = [self.spec(sha256="0" * 64), self.spec(owner_uid=os.geteuid() + 1),
                 self.spec(path=link), self.spec(path=self.directory),
                 self.spec(path=script, sha256=hashlib.sha256(script.read_bytes()).hexdigest())]
        for spec in cases:
            with patch("tools.template_build_windows.subprocess.Popen") as spawn:
                self.assert_code("helper_identity", lambda: WindowReducer(spec).reduce(fresh(), issue()))
                spawn.assert_not_called()
        # Metadata injection makes this portable to managed /tmp directories
        # that are not world-writable. Do not change any directory permissions.
        real_stat = os.fstat
        def writable_directory(descriptor):
            metadata = real_stat(descriptor)
            if stat.S_ISDIR(metadata.st_mode):
                return SimpleNamespace(st_uid=metadata.st_uid, st_mode=metadata.st_mode | 0o002)
            return metadata
        with patch("tools.template_build_windows.os.fstat", side_effect=writable_directory):
            self.assert_code("helper_identity", lambda: self.reducer().reduce(fresh(), issue()))
        parent_link = self.directory / "linked-parent"
        parent_link.symlink_to(self.binary.parent, target_is_directory=True)
        self.assert_code("helper_identity", lambda: self.reducer(path=parent_link / self.binary.name)
                         .reduce(fresh(), issue()))

    def test_exec_uses_checked_inode_even_if_path_is_replaced_after_hashing(self):
        path = self.directory / "replaced-helper"
        shutil.copy2(self.binary, path)
        other = self.directory / "replacement"
        shutil.copy2(self.fault, other)
        real_spawn = subprocess.Popen
        observed = []
        def replacing_spawn(*args, **kwargs):
            os.replace(other, path)
            observed.append(kwargs)
            return real_spawn(*args, **kwargs)
        with patch("tools.template_build_windows.subprocess.Popen", side_effect=replacing_spawn):
            decision = self.reducer(path=path).reduce(fresh(), issue())
        self.assertEqual(decision["event"]["Issued"]["id"], 0)
        self.assertEqual(observed[0]["env"], {"LC_ALL": "C"})
        self.assertTrue(observed[0]["executable"].startswith("/proc/self/fd/"))
        self.assertEqual(len(observed[0]["pass_fds"]), 1)
        self.assert_code("helper_identity", lambda: self.reducer(path=path).reduce(fresh(), issue()))

    def test_fault_output_timeout_and_blocked_input_reap_only_owned_child(self):
        cases = [(name, "invalid_response") for name in [
            "truncated", "duplicate", "unknown-field", "wrong-reason", "wrong-version",
            "bool-version", "bad-decision", "nonfinite", "extra-json", "non-utf8", "garbage"]]
        cases += [("stdout-flood", "output_too_large"), ("stderr-flood", "helper_stderr"),
                  ("exit", "helper_failed"), ("timeout", "helper_timeout"),
                  ("blocked-input", "helper_timeout")]
        real_spawn = subprocess.Popen
        for mode, code in cases:
            with self.subTest(mode=mode):
                observed = []
                def recording_spawn(*args, **kwargs):
                    child = real_spawn(*args, **kwargs)
                    observed.append(child)
                    return child
                spec = self.fault_spec(mode, timeout_seconds=0.1, max_output_bytes=128,
                                       max_input_bytes=1024*1024)
                state = fresh()
                if mode == "blocked-input":
                    # More than the pipe capacity, with no child reader.
                    state["open"] = [{"id": 0, "tenant": 7, "issued_at_ms": 0,
                                      "expires_at_ms": 10}] * 5000
                before = copy.deepcopy(state)
                with patch("tools.template_build_windows.subprocess.Popen", side_effect=recording_spawn):
                    self.assert_code(code, lambda: WindowReducer(spec).reduce(state, issue()))
                self.assertEqual(state, before)
                self.assertEqual(len(observed), 1)
                child = observed[0]
                self.assertIsNotNone(child.returncode)
                self.assertTrue(all(stream.closed for stream in (child.stdin, child.stdout, child.stderr)))
                with self.assertRaises(ChildProcessError):
                    os.waitpid(child.pid, os.WNOHANG)
