"""Original generated SDK RPCs for the tag fields absent from Commands.run."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import json
from connectrpc.errors import ConnectError

from e2b.envd.process import process_pb
from e2b.envd.utils import authentication_header, extract_start_pid
from e2b.sandbox_sync.commands.command_handle import CommandHandle
from e2b.sandbox_async.commands.command import first_event
from e2b.sandbox_async.commands.command_handle import AsyncCommandHandle
from sdk_command_input import call
from sdk_observations import value

KINDS = {'start': process_pb.StartRequest, 'connect': process_pb.ConnectRequest,
         'send_input': process_pb.SendInputRequest, 'close_stdin': process_pb.CloseStdinRequest,
         'send_signal': process_pb.SendSignalRequest}


class TagRpc:
    def __init__(self, calls, user):
        self.calls, self.commands = calls, calls.sandbox.commands
        self.headers = authentication_header(self.commands._envd_version, user)

    async def operation(self, *args, **kwargs):
        index = len(self.calls.rows)

        def record(error):
            self.calls.rows[index]['error']['rpc_code'] = value(error.code)
            self.calls.rows[index]['error']['rpc_details'] = [
                dict(type_name=detail.type_name, bytes=value(detail.message_bytes)) for detail in error.details]

        try:
            result = await self.calls.operation(*args, **kwargs)
        except ConnectError as error:
            record(error)
            raise
        if isinstance(result, ConnectError):
            record(result)
        return result

    async def stream(self, method, payload, capture, *, generation, label, seconds=60, expected=None,
                     allow_success=False):
        """With allow_success, the caller owns every returned original handle."""
        def opening(payload, timeout_ms):
            request = KINDS[method].from_json(json.dumps(payload))
            return getattr(self.commands._rpc, method)(request,
                headers=self.headers | {'keepalive-ping-interval': '50'}, timeout_ms=timeout_ms)

        def synchronous(payload, timeout_ms):
            events = opening(payload, timeout_ms)
            try:
                pid = extract_start_pid(next(events), method)
                return CommandHandle(pid, lambda: self.commands.kill(pid), events)
            except BaseException as error:
                try:
                    events.close()
                except BaseException as cleanup:
                    raise BaseExceptionGroup('tag stream opening and close failed', [error, cleanup])
                raise

        async def asynchronous(payload, timeout_ms):
            events = opening(payload, timeout_ms)
            try:
                pid = extract_start_pid(await first_event(events, 15), method)
                return AsyncCommandHandle(pid, lambda: self.commands.kill(pid), events,
                    on_stdout=capture.get('on_stdout'), on_stderr=capture.get('on_stderr'))
            except BaseException as error:
                try:
                    await events.aclose()
                except BaseException as cleanup:
                    raise BaseExceptionGroup('tag stream opening and close failed', [error, cleanup])
                raise

        result = await self.operation('process.' + method,
            asynchronous if self.calls.asynchronous else synchronous,
            payload, timeout_ms=seconds * 1000, expected=expected,
            allow_success=expected is not None,
            serialize=lambda handle: {'pid': handle.pid},
            evidence=dict(generation=generation, subscription=label))
        if expected is not None and not isinstance(result, ConnectError) and not allow_success:
            # An unexpected successful Connect only owns its subscription. Do
            # not signal the returned PID or lose its stream when validation fails.
            failure = AssertionError('expected tag RPC rejection returned a handle')
            try:
                await call(result.disconnect)
            except BaseException as cleanup:
                raise BaseExceptionGroup('unexpected tag handle and close failed', [failure, cleanup])
            raise failure
        return result

    async def unary(self, method, payload, *, generation, expected=None):
        async def invoke(payload, timeout_ms):
            request = KINDS[method].from_json(json.dumps(payload))
            response = await call(getattr(self.commands._rpc, method), request,
                                  headers=self.headers, timeout_ms=timeout_ms)
            return json.loads(response.to_json())
        return await self.operation('process.' + method, invoke, payload,
            timeout_ms=15000, expected=expected, evidence=dict(generation=generation))


class RecordedHandle:
    def __init__(self, calls, original, generation, label):
        self.calls, self.original = calls, original
        self.generation, self.label, self.pid = generation, label, original.pid

    async def wait(self, **kwargs):
        return await call(self.original.wait, **kwargs)

    async def disconnect(self):
        await self.calls.operation('handle.disconnect', self.original.disconnect,
            evidence=dict(generation=self.generation, subscription=self.label, handle_pid=self.pid))
