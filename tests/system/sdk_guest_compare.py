"""Paired original SDK command/file observations on isolated Go/Rust VMs.

Requires explicit immutable templates and the protected Rust release/config.
Creates only its own VMs through the existing gateway; never repairs a guest.
This command/file slice is not the complete #311 E03 or KVM acceptance.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import asyncio
import importlib.metadata
import json
import os
import shlex
import uuid

from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import AuthenticationException, FileNotFoundException, InvalidArgumentException
from e2b.sandbox.commands.command_handle import CommandExitException
from sdk_command_input import call, destroy_probes
from sdk_data_contract import guest_arguments, guest_candidate, observe_candidate
from sdk_data_samples import (COMMAND, ENV, FILES, NONZERO_COMMAND, file_entries,
                              validate_command, validate_content, validate_entries,
                              validate_file_info, validate_nonzero)
from sdk_server_smoke import options
from sdk_observations import CONTENT_LIMIT, Calls, Capture, value
from sdk_background_compare import background_case, comparable_background
from sdk_pty_compare import pty_case, comparable_pty
from sdk_file_compare import file_cases, sdk_file_cases, comparable_batch
from sdk_list_compare import list_case, comparable_list
from sdk_argument_compare import argument_cases, comparable_argument

FIXED_MTIME = 1_700_000_000


def plan():
    token = uuid.uuid4().hex
    return {'directory': '/tmp/sandproof-paired-' + token,
            'unknown_user': 'sp_missing_' + token[:16], 'users': ['root', 'ubuntu'],
            'mtime': FIXED_MTIME}


async def cases(sandbox, inputs, asynchronous, rows=None):
    observed = Calls(sandbox, asynchronous)
    if rows is not None:
        observed.rows = rows
    root = inputs['directory']
    await observed.invoke('commands.run', 'mkdir -m 0777 ' + shlex.quote(root)
                          + ' && ! getent passwd ' + shlex.quote(inputs['unknown_user']),
                          user='root', timeout=15)
    for user in inputs['users']:
        directory = root + '/' + user
        created = await observed.invoke('files.make_dir', directory, user=user, request_timeout=15)
        assert created is True
        repeated = await observed.invoke('files.make_dir', directory, user=user, request_timeout=15)
        assert repeated is False
        result = await observed.invoke('commands.run', COMMAND, cwd=directory, envs=ENV,
                                        user=user, timeout=15)
        validate_command(result, directory, user)
        result = await observed.invoke('commands.run', NONZERO_COMMAND, cwd=directory,
                                        user=user, timeout=15, expected=CommandExitException)
        validate_nonzero(result)
        written = await observed.invoke('files.write_files', file_entries(directory),
                                        user=user, request_timeout=15)
        assert {entry.path for entry in written} == {directory + '/' + name for name in FILES}
        await observed.invoke('files.write', directory + '/text.txt', FILES['text.txt'],
                              user=user, gzip=True, request_timeout=15)
        # Set actual guest timestamps identically before reading metadata. No
        # timestamps or directory sizes are removed from compared SDK results.
        await observed.invoke('commands.run', 'find ' + shlex.quote(directory)
                              + ' -exec touch -h -d @' + str(inputs['mtime']) + ' {} +',
                              user=user, timeout=15)
        entries = await observed.invoke('files.list', directory, depth=4, user=user, request_timeout=15)
        validate_entries(entries, directory)
        assert all(entry.modified_time.timestamp() == inputs['mtime'] for entry in entries)
        for name, content in FILES.items():
            path = directory + '/' + name
            info = await observed.invoke('files.get_info', path, user=user, request_timeout=15)
            validate_file_info(info, path, content)
            assert info.modified_time.timestamp() == inputs['mtime']
            read = await observed.invoke('files.read', path, format='bytes', user=user,
                                          gzip=True, request_timeout=15)
            validate_content(read, content)
            validate_content(await observed.stream(path, user), content)
        moved = directory + '/moved.txt'
        await observed.invoke('files.rename', directory + '/text.txt', moved, user=user, request_timeout=15)
        read = await observed.invoke('files.read', moved, format='bytes', user=user, request_timeout=15)
        validate_content(read, FILES['text.txt'])
        await observed.invoke('files.remove', moved, user=user, request_timeout=15)
        await observed.invoke('files.get_info', moved, user=user, request_timeout=15,
                              expected=FileNotFoundException)
        await observed.invoke('commands.run', 'printf forbidden', cwd=directory + '/missing',
                              user=user, timeout=15, expected=InvalidArgumentException)
    await observed.invoke('commands.run', 'printf forbidden', user=inputs['unknown_user'],
                          timeout=15, expected=AuthenticationException)
    await file_cases(observed, inputs)
    await argument_cases(observed, inputs)
    for user in inputs['users']:
        for mode in ('direct', 'reconnect-exit', 'kill'):
            await background_case(observed, user, root + '/' + user, mode)
        identity = await observed.invoke('commands.run', 'id -u ' + shlex.quote(user), user='root', timeout=20)
        assert identity.exit_code == 0 and not identity.stderr and identity.stdout.strip().isdecimal()
        for mode in ('direct', 'reconnect-exit', 'kill'):
            await pty_case(observed, user, int(identity.stdout.strip()), root + '/' + user, mode)
    await list_case(observed, inputs)
    healthy = await observed.invoke('commands.run', 'printf paired-healthy', user='ubuntu', timeout=15)
    assert (healthy.stdout, healthy.stderr, healthy.exit_code) == ('paired-healthy', '', 0)
    return observed.rows


def comparable(rows):
    """Stream segmentation and validated background PID references may differ."""
    result = []
    for original in rows:
        assert original.get('status', 'complete') == 'complete', 'incomplete call is not comparable'
        row = {key: part for key, part in original.items() if key != 'transport_chunk_bytes'}
        if row['method'] == 'background.case':
            row = comparable_background(row)
        if row['method'] == 'pty.case':
            row = comparable_pty(row)
        if row['method'] == 'files.octet_batch':
            row = comparable_batch(row)
        if row['method'] == 'commands.list_case':
            row = comparable_list(row)
        if row['method'] == 'commands.argument_case':
            row = comparable_argument(row)
        if 'callbacks' in row:
            row['callbacks'] = {key: ''.join(parts) for key, parts in row['callbacks'].items()}
        if row['method'] == 'files.list' and 'result' in row:
            entries = row['result']
            assert len({entry['path'] for entry in entries}) == len(entries), 'duplicate listed path'
        result.append(row)
    return result


def compare(left, right, label):
    left, right = comparable(left), comparable(right)
    assert len(left) == len(right), label + ': call count differs'
    for index, (a, b) in enumerate(zip(left, right)):
        # JSON distinguishes bool from int, unlike Python's True == 1.
        assert json.dumps(a, sort_keys=True) == json.dumps(b, sort_keys=True), (
            label + ': call ' + str(index) + ' differs (' + a['method'] + ')')


async def paired(cls, owner, inputs, go, rust, templates, item):
    from tools import guest_probe
    live, failure = [], None
    try:
        for kind, selected in templates:
            attempt = {'kind': kind, 'template': selected, 'outcome': 'unknown'}
            item['create_attempts'].append(attempt)
            sandbox = await call(cls.create, template=selected, timeout=300,
                                  allow_internet_access=False, **owner)
            attempt.update(outcome='returned', sandbox_id=sandbox.sandbox_id)
            assert all(sandbox.sandbox_id != known.sandbox_id for known in live), 'repeated VM identity'
            live.append(sandbox)
            entry = {'kind': kind, 'template': selected, 'sandbox_id': sandbox.sandbox_id}
            item['pair'].append(entry)
            async def observe():
                if kind == 'rust':
                    return await observe_candidate(sandbox, rust, asynchronous=cls is AsyncSandbox)
                result = await call(sandbox.commands.run, guest_probe.go_command(go['file']),
                                    user='root', timeout=20)
                return {'identity': go, 'observation': guest_probe.go_receipt(result, go['file'])}
            entry['before'] = await observe()
            entry['calls'] = []
            entry['calls'] = await cases(sandbox, inputs, cls is AsyncSandbox, entry['calls'])
            entry['sdk_files'] = []
            await sdk_file_cases(sandbox, inputs, cls is AsyncSandbox, entry['sdk_files'])
            entry['after'] = await observe()
            assert entry['after'] == entry['before'], kind + ' daemon changed during paired cases'
        compare(item['pair'][0]['calls'], item['pair'][1]['calls'], 'Go/Rust')
        compare(item['pair'][0]['sdk_files'], item['pair'][1]['sdk_files'], 'Go/Rust SDK-specific octet batches')
    except BaseException as error:
        failure = error
    try:
        await destroy_probes(live)
        item['known_destroy_acknowledged'] = True
    except BaseException as cleanup:
        item['known_destroy_acknowledged'] = False
        if failure is not None:
            raise BaseExceptionGroup('paired probe and known-VM cleanup failed', [failure, cleanup])
        raise
    if failure is not None:
        raise failure


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    template, rust = guest_candidate(args)
    assert rust is not None and args.go_template and template != args.go_template
    from tools import guest_probe
    go = guest_probe.load_go_expected()
    inputs = plan()  # One immutable input set for both implementations and SDK modes.
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    report = {'schema_version': 1, 'sdk_version': '2.51.0', 'inputs': inputs, 'results': results,
        'scope': 'paired foreground/background commands and PTY, stdin, resize, same-PID reconnect, exit/kill and retained End; '
                 'cwd/account errors, callbacks and unary/batch/gzip/stream files; '
                 'file user/home/ownership, path errors and multipart partial writes; '
                 'octet partial batches compared Go/Rust within each SDK mode, '
                 'bounded transient file observations retained separately; '
                 'complete controlled command/PTY List snapshots and known-entry retirement; '
                 'foreground/background cwd and user rejections with absent side effects and healthy follow-ups; '
                 'not full fault/resource matrices or E03/E06 acceptance'}
    try:
        for cls in (Sandbox, AsyncSandbox):
            item = {'sdk': cls.__name__, 'pair': [], 'create_attempts': []}
            results.append(item)
            await paired(cls, owner, inputs, go, rust,
                         [('go', args.go_template), ('rust', template)], item)
        compare(results[0]['pair'][0]['calls'], results[1]['pair'][0]['calls'], 'sync/async')
        report['status'] = 'pass'
    except BaseException as error:
        report.update(status='failed', failure=type(error).__name__)
        raise
    finally:
        # Preserve both observed sides even on a comparison/cleanup failure.
        # No API key, access token, connection header or host config is printed.
        print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--go-template', required=True, help='immutable template using the pinned base Go envd')
    guest_arguments(parser, required=True)
    asyncio.run(exercise(parser.parse_args()))
