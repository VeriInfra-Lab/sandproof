"""Run standalone Inbox Lookup with retirement of its two private extraction targets."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys

from tools import formal_builds
from tools.system_acceptance import digest, json_file, private_path, write_once


ROOT = Path(__file__).resolve().parents[1]
DRIVER = ROOT/'verification/experiments/stage1-protocol/inbox-lookup/module_acceptance.py'
CASES = ('correct', 'key-only')
TARGETS = tuple(Path(f'module/formal-{case}/source/artifacts/extract-target') for case in CASES)
INVOCATION = 'standalone-inbox-lookup.json'


def lookup():
    spec = importlib.util.spec_from_file_location('standalone_lookup_retirement', DRIVER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def checked_file(path, files, expected=None):
    if not path.is_absolute() or path.resolve(strict=True) != path or not path.is_file():
        raise ValueError('original evidence must be a regular file: ' + str(path))
    checksum = digest(path)
    if expected is not None and checksum != expected:
        raise ValueError('original evidence changed: ' + str(path))
    files[path] = checksum
    return path


def proof_files(module, saved_view, variant, folder, candidate, native, files, *, permitted=None):
    """Read the original physical-file audit and the completed type check without Lean."""
    record = json_file(checked_file(folder/'result.json', files, saved_view['result_sha256']), limit=32 * 1024**2)
    project = folder/'source'
    rust_variant, library, name = module.VARIANTS[variant]
    theorem = 'SandboxModel.Stage1Protocol.' + name
    actual = record['actual']
    if (record.get('kind') != 'inbox-lookup-formal-development' or record.get('complete') is not True
            or record.get('source_commit') != candidate['commit'] or record.get('variant') != variant
            or saved_view.get('theorem') != theorem or saved_view.get('property_rejected') is not (variant == 'key-only')
            or saved_view.get('rust_formal_inputs_match') is not True or saved_view.get('registry_content_match') is not True
            or theorem not in actual['audits'] or set(record['llbc']) != {'original', 'candidate'}
            or native['variants'][rust_variant]['inputs'][module.MODEL] != saved_view['candidate_model_sha256']
            or record['candidate_model_sha256'] != saved_view['candidate_model_sha256']):
        raise ValueError('original Lookup proof or Rust binding differs: ' + variant)
    if permitted is not None and (
            module.proposed_changes(native['inputs'], record['inputs']) != set(permitted)
            or saved_view.get('rust_formal_documented_source_differences') != permitted):
        raise ValueError('original Lookup proof inputs differ from the local-only change set: ' + variant)
    checked_file(project/module.MODEL, files, saved_view['candidate_model_sha256'])
    for path, expected in actual['files'].items():
        checked_file(project/path, files, expected)
    for label, ir in record['llbc'].items():
        if ir['path'] != f'artifacts/{label}.llbc':
            raise ValueError('unexpected original Lookup LLBC path')
        checked_file(project/ir['path'], files, ir['sha256'])
    for path, expected in record['raw_generated'].items():
        checked_file(folder/'generated'/path, files, expected)
    for command in record['commands']:
        label = command['label']
        if not re.fullmatch('[a-z0-9-]+', label) or command['exit_code'] != 0:
            raise ValueError('original Lookup proof command did not succeed')
        checked_file(folder/(label+'.log'), files, command['log_sha256'])
    physical = folder/'physical-files'
    checked_file(physical/'service.log', files, actual['service_log_sha256'])
    checked_file(physical/'requests.jsonl', files)
    checked_file(folder/'actual-service.stdout', files)
    checked_file(folder.parent/(folder.name+'.log'), files)
    module.registration(ROOT)._saved_physical_view(actual, project, physical, record['commands'])
    proposition = ('¬ ' if variant == 'key-only' else '') + module.THEOREM_TYPE + module.LOOKUP
    expected = f'import SandboxModel.Lifecycle.Protocol.{library}\n#check ({theorem} : {proposition})\n'
    check = checked_file(project/'artifacts/ModuleAcceptance.lean', files, saved_view['contract_type_source_sha256'])
    log = checked_file(folder/'contract-type.log', files, saved_view['contract_type_log_sha256'])
    if check.read_text() != expected or not log.stat().st_size:
        raise ValueError('original successful Lookup theorem type check differs: ' + variant)


def checked_local_plan(result, invocation, module):
    """Read the original planner again against the enrolled candidate, without running checks."""
    candidate = invocation['candidate']
    base, paths = module.changed_files(invocation['base'], candidate['commit'])
    impact = module.local_lookup_impact(paths, base)
    plan = result['plan']
    if (plan.get('status') != 'lookup-only' or plan.get('base_commit') != base
            or plan.get('changed_paths') != paths or plan.get('native_reuse_paths') != sorted(paths)
            or plan.get('checks') != ['reuse-original-native', 'formal-correct', 'formal-key-only']
            or plan.get('pending_downstream') != impact['pending_jobs']
            or plan.get('unmapped_downstream') != impact['unmapped']
            or result['native'].get('source_differences') != sorted(paths)
            or result['native'].get('original_candidate', {}).get('commit') != base):
        raise ValueError('original Lookup-only scope or pending downstream list differs')
    return sorted(paths)


def stage_files(output, invocation, terminal, module):
    files = {}
    checked_file(output/INVOCATION, files)
    local = invocation.get('lookup_only', False)
    module_receipt = output/'module/module-result.json'
    local_receipt = output/'module/lookup-local-result.json'
    path, unrelated = (local_receipt, module_receipt) if local else (module_receipt, local_receipt)
    if os.path.lexists(unrelated):
        raise ValueError('Lookup receipt belongs to a different acceptance scope')
    code = terminal['returncode']
    if local and code == 0:
        raise ValueError('Lookup-only cannot have a full module exit status')
    if code != (2 if local else 0):
        if code in (-signal.SIGKILL, 128 + signal.SIGKILL):
            raise ValueError('unknown abruptly stopped Lookup executor')
        if os.path.lexists(path):
            result = json_file(checked_file(path, files))
            if (result.get('candidate') != invocation['candidate'] or result.get('complete') is not False
                    or result.get('lookup_only_complete') is True):
                raise ValueError('Lookup failure differs from its original module report')
        # The shared helper binds the actual waited exit and raw executor log.
        return files

    result = json_file(checked_file(path, files))
    candidate = invocation['candidate']
    expected_kind = 'inbox-lookup-only-proof' if local else 'stage1-module-acceptance'
    if (result.get('schema') != 1 or result.get('kind') != expected_kind
            or result.get('module') != module.MODULE or result.get('complete') is not (not local)
            or result.get('full_stage1_acceptance') is not False or result.get('candidate') != candidate
            or set(result.get('formal', ())) != set(CASES)
            or local and (result.get('lookup_only_complete') is not True or result.get('fresh_runs') is not False)
            or not local and result.get('lookup_only_complete') is True):
        raise ValueError('original completed Lookup receipt for the enrolled scope required')
    prepared = invocation['native']
    if local and prepared is None:
        raise ValueError('Lookup-only requires its original scoped native receipt')
    native_use = result['native']
    receipt = Path(native_use['receipt'])
    wanted = (output/'module'/module.NATIVE_RECEIPT if prepared is None else Path(prepared['path']))
    if (receipt != wanted or native_use['use'] != ('RUN' if prepared is None else 'REUSE')
            or prepared is not None and native_use['receipt_sha256'] != prepared['sha256']):
        raise ValueError('Lookup result did not use its original native receipt')
    original = json_file(checked_file(receipt, files, native_use['receipt_sha256']), limit=16 * 1024**2)
    base = candidate if prepared is None else native_use['original_candidate']
    native, admitted = module.load_native_reuse(receipt, native_use['receipt_sha256'], base, original['key'], [])
    if admitted['result_sha256'] != native_use['result_sha256']:
        raise ValueError('original Lookup native result differs')
    if local and original['key'].get('scoped_native_tools') is not True:
        raise ValueError('original Lookup native receipt was not prepared for scoped reuse')
    permitted = checked_local_plan(result, invocation, module) if local else None
    checked_file(receipt.parent/'original-native/result.json', files, original['result_sha256'])
    checked_file(receipt.parent/'original-native.log', files, original['launcher_log_sha256'])
    for row in native['commands']:
        checked_file(receipt.parent/'original-native'/row['variant']/(row['label']+'.log'), files, row['log_sha256'])
    for variant in CASES:
        proof_files(module, result['formal'][variant], variant, path.parent/f'formal-{variant}',
                    candidate, native, files, permitted=permitted)
    return files


def closeout(output, observer):
    module = lookup()
    _, enrollment, targets = formal_builds.admission(output)
    invocation = json_file(private_path(output/INVOCATION))
    if (invocation.get('schema') != 1 or invocation['candidate'] != module.clean_candidate()
            or type(invocation.get('lookup_only', False)) is not bool
            or enrollment['candidate'] != invocation['candidate']['commit']
            or targets != [output/target for target in TARGETS]):
        raise ValueError('original Lookup candidate and two exact extraction targets required')
    return formal_builds.retire(output, observer, lambda terminal: stage_files(output, invocation, terminal, module))


def run(output, observer, base, formal_lock, native=None, *, lookup_only=False):
    if output.is_relative_to(ROOT) or ROOT.is_relative_to(output) or any(output.iterdir()):
        raise ValueError('standalone Lookup requires an empty private run directory outside its checkout')
    if observer.is_relative_to(ROOT):
        raise ValueError('host observer must be installed outside the proof checkout')
    if lookup_only and native is None:
        raise ValueError('Lookup-only requires the original scoped native receipt and SHA256')
    candidate = lookup().clean_candidate()
    formal_builds.enroll(output, candidate=candidate['commit'], observer=observer, targets=TARGETS)
    write_once(output/INVOCATION, dict(schema=1, candidate=candidate, native=native,
                                       base=base, formal_lock=str(formal_lock), lookup_only=lookup_only))
    args = [sys.executable, str(DRIVER), '--base', base, '--output', str(output/'module'),
            '--formal-lock', str(formal_lock)]
    if native is not None:
        args.extend(('--reuse-native', native['path'], '--reuse-native-sha256', native['sha256']))
    if lookup_only:
        args.append('--lookup-only')
    terminal = formal_builds.execute(output, args, cwd=ROOT)
    code = terminal['returncode']
    try:
        print(json.dumps(closeout(output, observer), sort_keys=True))
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f'Lookup build closeout pending in {output}: {error}', file=sys.stderr)
        if code == 0:
            return 75
    return code if code >= 0 else 128-code


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('run', 'closeout'))
    parser.add_argument('--output', type=Path, required=True, help='owner-private run directory')
    parser.add_argument('--observer', type=Path, required=True)
    parser.add_argument('--base')
    parser.add_argument('--formal-lock', type=Path)
    parser.add_argument('--reuse-native', type=Path)
    parser.add_argument('--reuse-native-sha256')
    parser.add_argument('--lookup-only', action='store_true', help='check both Lookup proofs and keep downstream work pending')
    args = parser.parse_args(argv)
    os.umask(0o077)
    output = private_path(args.output, directory=True)
    observer = args.observer.resolve(strict=True)
    if args.mode == 'closeout':
        if args.lookup_only or any(value is not None for value in (
                args.base, args.formal_lock, args.reuse_native, args.reuse_native_sha256)):
            parser.error('closeout reads the original run and never executes Rust or Lean')
        print(json.dumps(closeout(output, observer), sort_keys=True))
        return 0
    if (args.base is None or args.formal_lock is None
            or (args.reuse_native is None) != (args.reuse_native_sha256 is None)):
        parser.error('run requires --base, --formal-lock and both original native receipt parameters when reusing')
    native = None if args.reuse_native is None else dict(
        path=str(args.reuse_native.absolute()), sha256=args.reuse_native_sha256)
    return run(output, observer, args.base, args.formal_lock.absolute(), native, lookup_only=args.lookup_only)


if __name__ == '__main__':
    raise SystemExit(main())
