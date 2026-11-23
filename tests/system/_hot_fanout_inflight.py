"""Observe original stock SDK branch streams through revocation and port reuse."""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import secrets
import shlex
import time

from lifecycle_unload_data_drain import _live_routes
from lifecycle_unload_runtime import original_process


class OriginalBranchStreams:
    def __init__(self, client, sandbox, native, context, port, capture):
        from transfer_interruption import PausedUpload

        self.client, self.sandbox, self.native = client, sandbox, native
        self.context, self.port, self.capture = context, port, capture
        prefix = "/tmp/sandproof-fanout-inflight-" + secrets.token_hex(8)
        self.marker, self.gate, self.upload = (prefix + suffix for suffix in (".once", ".fifo", ".upload"))
        self.producer = PausedUpload()
        self.handle = None

    def routes(self):
        return _live_routes(self.native, {"context": self.context}, branch_port=self.port)

    def route_snapshot(self, count, required=()):
        deadline = time.monotonic() + min(8, self.client.remaining())
        while True:
            routes = self.routes()
            if len(routes) == count and set(required) <= routes.keys():
                return routes
            if time.monotonic() >= deadline:
                raise TimeoutError("exact live command/upload route cohort was not observed")
            time.sleep(min(.025, self.client.remaining()))

    def start(self, executor):
        from e2b.exceptions import FileNotFoundException
        from transfer_interruption import UPLOAD_PREFIX

        self.route_snapshot(0)
        marker, gate = map(shlex.quote, (self.marker, self.gate))
        command = (f"set -eu; mkfifo {gate}; printf 'once\\n' >> {marker}; "
                   f"printf 'ready\\n'; read -r release < {gate}; printf 'unexpected-completion\\n'")
        self.handle = self.sandbox.commands.run(command, user="root", background=True,
                                                timeout=65, request_timeout=55)
        stdout = ""
        for chunk, _stderr, _pty in self.handle:
            stdout += chunk or ""
            if stdout == "ready\n":
                break
            assert len(stdout) <= len("ready\n")
        else:
            raise AssertionError("original SDK command ended before the FIFO barrier")
        command_routes = self.route_snapshot(1)
        self.transfer = executor.submit(self.sandbox.files.write, self.upload, self.producer,
                                        user="root", request_timeout=55, use_octet_stream=True)
        assert self.producer.blocked.wait(min(8, self.client.remaining()))
        self.original_routes = self.route_snapshot(2, command_routes)
        deadline = time.monotonic() + min(8, self.client.remaining())
        while True:
            assert not self.transfer.done(), "SDK upload settled before revocation"
            try:
                size = self.sandbox.files.get_info(self.upload, user="root",
                                                   request_timeout=min(3, self.client.remaining())).size
            except FileNotFoundException:
                size = 0
            if size:
                assert 0 < size <= UPLOAD_PREFIX
                break
            if time.monotonic() >= deadline:
                raise TimeoutError("actual guest upload prefix was not observed")
            time.sleep(min(.025, self.client.remaining()))
        assert self.sandbox.files.read(self.marker, user="root",
                                      request_timeout=min(3, self.client.remaining())) == "once\n"
        self.route_snapshot(2, self.original_routes)
        assert not self.transfer.done()
        self.capture.update(original_sdk_exec_pid=self.handle.pid, original_live_relays=2,
                            original_exec_side_effect="once", guest_upload_prefix_bytes=size)

    def wait_drained(self):
        deadline = time.monotonic() + min(15, self.client.remaining())
        sources = [record["process"] for _, record in self.original_routes.values()]
        while any(original_process(source) for source in sources) or self.routes():
            if time.monotonic() >= deadline:
                raise TimeoutError("original branch transport did not drain")
            time.sleep(min(.025, self.client.remaining()))
        self.capture["original_relays_exited_before_refill"] = True

    def finish_after_reuse(self, replacement):
        from e2b.exceptions import AuthenticationException, SandboxNotFoundException
        from e2b.sandbox.commands.command_handle import CommandExitException
        from gateway_stream_limits import stream_failure
        from transfer_interruption import UPLOAD_PREFIX, upload_failure

        # The original producer never supplied the tail while the branch slot
        # was being reclaimed. Finish the same outstanding client call after
        # reuse so no buffered old bytes can be mistaken for new branch traffic.
        assert self.producer.sent == UPLOAD_PREFIX
        self.producer.release.set()
        try:
            self.transfer.result(timeout=min(12, self.client.remaining()))
        except Exception as error:
            if not upload_failure(error):
                raise
            self.capture["original_upload_result"] = type(error).__name__
        else:
            raise AssertionError("revoked original SDK upload completed successfully")

        try:
            self.handle.wait()
        except CommandExitException as error:
            assert error.exit_code != 0 and "unexpected-completion" not in error.stdout
            self.capture["original_exec_result"] = type(error).__name__
        except Exception as error:
            if not (stream_failure(error) or type(error) in (AuthenticationException, SandboxNotFoundException)):
                raise
            self.capture["original_exec_result"] = type(error).__name__
        else:
            raise AssertionError("revoked original SDK command completed successfully")
        paths = (self.marker, self.gate, self.upload)
        check = " && ".join("test ! -e " + shlex.quote(path) for path in paths)
        result = replacement.commands.run(check, user="root", timeout=min(10, self.client.remaining()),
                                          request_timeout=min(12, self.client.remaining()))
        assert result.exit_code == 0
        self.capture.update(no_original_side_effect_in_replacement=True,
                            driver_sent_no_command_or_upload_replay=True)


@contextmanager
def observe_inflight_branch(client, sandbox, native, context, port, report):
    streams = OriginalBranchStreams(client, sandbox, native, context, port, report)
    with ThreadPoolExecutor(max_workers=1, thread_name_prefix="hot-fanout-original-upload") as executor:
        try:
            streams.start(executor)
            yield streams
        finally:
            streams.producer.release.set()
            if streams.handle is not None:
                # Close this original subscription locally. Parent cleanup
                # owns all remaining guest work; no second Start is issued.
                streams.handle.disconnect()
