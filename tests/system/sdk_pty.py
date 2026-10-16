"""Original fixed SDK PTY calls against a selected, already published template.

Requires /bin/bash, /usr/bin/python3, root and ubuntu in the test guest.
Creates only its own instances; candidate execution needs isolated-host approval.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name('_assertions_required.py'))
)["require_assertions"]
_require_assertions()

import argparse
import asyncio
import base64
import hashlib
import importlib.metadata
import json
import os
import shlex
import struct
import uuid

from e2b import AsyncSandbox, Sandbox
from e2b.sandbox.commands.command_handle import CommandExitException, PtySize
from sdk_command_input import call, destroy_probes
from sdk_data_contract import guest_arguments, guest_candidate, observe_candidate
from sdk_server_smoke import options

PAYLOAD = bytes(range(256)) * 9
HEADER = b'\x00sandproof-pty\x00'
READY_HEADER = b'\x00sandproof-pty-ready\x00'
OUTPUT_LIMIT = 128 * 1024
VALUE = 'PTY value 中文'

# Executed by exec inside the SDK's original interactive login shell. The FIFO
# handshake proves raw mode and the marker are ready before input/resize/kill.
# No chdir/setuid here: those observations must come from the original Start.
GUEST_PROGRAM = r'''
import base64,fcntl,json,os,struct,sys,termios,tty
c=json.loads(base64.b64decode(sys.argv[1]))
def size():
    return list(struct.unpack('HHHH',fcntl.ioctl(0,termios.TIOCGWINSZ,b'\0'*8))[:2])
def write(fd,data):
    while data:
        n=os.write(fd,data)
        if n<=0: raise RuntimeError('short PTY write')
        data=data[n:]
tty.setraw(0)
ready=dict(pid=os.getpid(),uid=os.getuid(),cwd=os.getcwd(),
           value=os.environ.get('SANDPROOF_PTY_VALUE'),
           size=size(),tty=[os.isatty(fd) for fd in (0,1,2)])
with open(c['directory']+'/started','xb') as f: f.write(b'once\n')
if c.get('report_ready'): write(1,b'\0sandproof-pty-ready\0')
with open(c['directory']+'/ready','wb',buffering=0) as f:
    f.write(json.dumps(ready).encode()+b'\n')
data=b''
while len(data)<c['length']:
    part=os.read(0,c['length']-len(data))
    if not part: raise RuntimeError('unexpected PTY EOF')
    data+=part
body=json.dumps(dict(ready=ready,resized=size())).encode()
frame=b'\0sandproof-pty\0'+struct.pack('!I',len(body))+body+data
cut=len(frame)//2
write(1,frame[:cut])
write(2,frame[cut:])
sys.exit(c['exit_code'])
'''


def configuration(directory, exit_code):
    return dict(directory=directory, length=len(PAYLOAD), exit_code=exit_code)


def bootstrap(config):
    argument = base64.b64encode(json.dumps(config).encode()).decode()
    line = ('exec /usr/bin/python3 -I -u ' + shlex.quote(config['directory'] + '/program.py') + ' ' + argument + '\n').encode()
    assert len(line) < 512, 'bootstrap exceeds the probe canonical input bound'
    return line


class Capture:
    def __init__(self):
        self.data = bytearray()

    def __call__(self, chunk):
        if not isinstance(chunk, bytes) or len(self.data) + len(chunk) > OUTPUT_LIMIT:
            raise ValueError('PTY output exceeds the probe bound or is not bytes')
        self.data.extend(chunk)


def validate_frame(data, ready, resized):
    prefix, separator, frame = bytes(data).partition(HEADER)
    assert separator and len(frame) >= 4, 'missing PTY frame'
    length = struct.unpack('!I', frame[:4])[0]
    assert 0 < length <= 4096, 'invalid PTY metadata length'
    assert len(frame) == 4 + length + len(PAYLOAD), 'truncated or extra PTY data'
    assert json.loads(frame[4:4 + length]) == dict(ready=ready, resized=resized)
    assert frame[4 + length:] == PAYLOAD, 'PTY binary bytes changed'
    return dict(received_bytes=len(PAYLOAD), sha256=hashlib.sha256(PAYLOAD).hexdigest(),
                startup_bytes=len(prefix), startup_sha256=hashlib.sha256(prefix).hexdigest())


async def wait_result(handle, capture, asynchronous, expected):
    try:
        result = await call(handle.wait, **({} if asynchronous else {'on_pty': capture}))
    except CommandExitException as error:
        result = error
    assert (result.exit_code, result.error, result.stdout, result.stderr) == (*expected, '', '')
    return dict(exit_code=result.exit_code, error=result.error)


async def close_case(pty, pid, handles, *, finished, kill_attempted):
    """Keep every cleanup error and the original exception as its context."""
    errors = []
    if pid is not None and not finished and not kill_attempted:
        try:
            # False means already absent; the owning VM is still destroyed by
            # exercise. A signal acknowledgement is not a resource audit.
            await call(pty.kill, pid, request_timeout=15)
        except BaseException as error:
            errors.append(error)
    for handle in reversed(handles):
        try:
            await call(handle.disconnect)
        except BaseException as error:
            errors.append(error)
    if errors:
        raise BaseExceptionGroup('known PTY cleanup failed', errors)


async def pty_case(sandbox, user, uid, mode, asynchronous):
    directory = '/tmp/pty-' + uuid.uuid4().hex
    setup = await call(sandbox.commands.run,
        'mkdir -m 0700 ' + shlex.quote(directory) + ' && mkfifo -m 0600 ' + shlex.quote(directory + '/ready'),
        user=user, timeout=20)
    assert (setup.exit_code, setup.stdout, setup.stderr) == (0, '', '')
    await call(sandbox.files.write, directory + '/program.py', GUEST_PROGRAM, user=user, request_timeout=15)
    # Wide values exercise the fixed Go uint16 narrowing without allocating a
    # large terminal; the guest must observe the low sixteen bits.
    wide = 65536 if mode == 'reconnect-exit' else 0
    initial = PtySize(rows=wide + 24, cols=wide + 80)
    resized = PtySize(rows=wide + 30, cols=wide + 100)
    expected = (-1, 'signal: killed') if mode == 'kill' else ((17, 'exit status 17') if wide else (0, None))
    capture, handles, pid = Capture(), [], None
    finished = kill_attempted = False
    try:
        handle = await call(sandbox.pty.create, size=initial, user=user, cwd=directory,
            envs={'SANDPROOF_PTY_VALUE': VALUE}, timeout=60, request_timeout=15,
            **({'on_data': capture} if asynchronous else {}))
        handles.append(handle)
        assert type(handle.pid) is int and 0 < handle.pid <= 4194304
        pid = handle.pid
        await call(sandbox.pty.send_stdin, pid, bootstrap(configuration(directory, 17 if wide else 0)), request_timeout=15)
        observed = await call(sandbox.commands.run, 'cat ' + shlex.quote(directory + '/ready'), user=user, timeout=20)
        assert observed.exit_code == 0 and not observed.stderr and len(observed.stdout) <= 4096
        ready = dict(pid=pid, uid=uid, cwd=directory, value=VALUE, size=[24, 80], tty=[True]*3)
        assert json.loads(observed.stdout) == ready, 'PTY Start identity, directory, environment or initial size differs'
        assert pid in [item.pid for item in await call(sandbox.commands.list, request_timeout=15)]
        if mode == 'reconnect-exit':
            handles.remove(handle)
            await call(handle.disconnect)
            capture = Capture()
            handle = await call(sandbox.pty.connect, pid, timeout=60, request_timeout=15,
                                **({'on_data': capture} if asynchronous else {}))
            handles.append(handle)
            assert handle.pid == pid, 'PTY reconnect changed PID'
        if mode == 'kill':
            kill_attempted = True
            killed = await call(sandbox.pty.kill, pid, request_timeout=15)
            assert killed is True
        else:
            await call(sandbox.pty.resize, pid, resized, request_timeout=15)
            for chunk in (b'', PAYLOAD[:513], PAYLOAD[513:]):
                await call(sandbox.pty.send_stdin, pid, chunk, request_timeout=15)
        terminal = await wait_result(handle, capture, asynchronous, expected)
        finished = True
        # Connect immediately while the fixed envd's terminal record is retained.
        # No command is reissued; late Connect must contain End and no history.
        late_capture = Capture()
        late = await call(sandbox.pty.connect, pid, timeout=20, request_timeout=15,
                          **({'on_data': late_capture} if asynchronous else {}))
        handles.append(late)
        assert late.pid == pid
        late_result = await wait_result(late, late_capture, asynchronous, expected)
        assert late_result == terminal
        assert not late_capture.data, 'late Connect replayed PTY output'
        # Fixed Go broadcasts End before its OnExit retention/removal hook.
        # Record this observation without inventing synchronous List removal.
        listed_after_end = pid in [item.pid for item in await call(sandbox.commands.list, request_timeout=15)]
        assert await call(sandbox.files.read, directory + '/started', user=user, request_timeout=15) == 'once\n'
        binary = validate_frame(capture.data, ready, [30, 100]) if mode != 'kill' else {}
        if mode == 'kill':
            assert HEADER not in capture.data, 'killed input waiter emitted a completed frame'
        return dict(user=user, mode=mode, pid=pid, start_markers=1, terminal=terminal,
                    listed_after_end=listed_after_end,
                    requested_size=[initial.rows, initial.cols], observed_size=ready['size'],
                    **({} if mode == 'kill' else {'requested_resize': [resized.rows, resized.cols],
                                                'observed_resize': [30, 100]}), **binary)
    finally:
        await close_case(sandbox.pty, pid, handles, finished=finished, kill_attempted=kill_attempted)


async def user_cases(sandbox, user, asynchronous):
    # Ask root for the named user's UID, not the same possibly broken user switch.
    result = await call(sandbox.commands.run, 'id -u ' + shlex.quote(user), user='root', timeout=20)
    assert result.exit_code == 0 and not result.stderr and result.stdout.strip().isdecimal()
    uid = int(result.stdout.strip())
    return [await pty_case(sandbox, user, uid, mode, asynchronous)
            for mode in ('direct', 'reconnect-exit', 'kill')]


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    template, candidate = guest_candidate(args)
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template=template, timeout=300, allow_internet_access=False, **owner)
        try:
            before = await observe_candidate(sandbox, candidate, asynchronous=cls is AsyncSandbox)
            cases = []
            for user in ('root', 'ubuntu'):
                cases.extend(await user_cases(sandbox, user, cls is AsyncSandbox))
            after = await observe_candidate(sandbox, candidate, asynchronous=cls is AsyncSandbox)
            assert before == after, 'Rust guest identity changed during PTY cases'
            row = dict(sdk=cls.__name__, cases=cases)
            if candidate is not None:
                row.update(sandbox_id=sandbox.sandbox_id, rust_guest=before)
            results.append(row)
        finally:
            await destroy_probes([sandbox])
    report = dict(sdk_version='2.51.0', results=results,
        scope='public PTY SDK calls, raw binary input/merged output, resize, known-PID reconnect, exit/kill and retained End; no full timeout, fault, process-tree or paired Go/Rust acceptance')
    if getattr(args, 'template', None) is not None:
        report['template'] = template
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    guest_arguments(parser)
    asyncio.run(exercise(parser.parse_args()))
