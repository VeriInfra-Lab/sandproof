"""Reproduce the isolated hot branch close and occupancy proof component."""

import argparse
from hashlib import sha256
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile

import blob_provider
from source_profile import linux_admission, source_profile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
CURRENT = Path('verification/experiments/create-transaction-link/current')
VIEW = Path('verification/aeneas/hot-branch-close')
LIB = 'SandproofBranchBatch'
AUDIT = re.compile(r'^#audit_controller_proof (\S+)$', re.M)


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def replay(args):
    inventory = json.loads((HERE / 'inventory.json').read_bytes())
    checkout = args.checkout.resolve(strict=True)
    output = args.output.resolve()
    admission = {}
    require(not args.with_blob_bridge or args.approved_linux,
            'the original BLOB provider was reviewed for the exact Linux inputs')
    require(not args.with_branch_scan or (args.approved_linux and args.with_window),
            'the branch scan view requires the validated Linux retirement capture')
    require(not args.with_load_key or (args.approved_linux and args.with_window),
            'the load-key view requires the validated Linux retirement capture')
    if args.approved_linux:
        require(args.with_window, 'the exact Linux replay requires the CloseWindow case')
        admission = linux_admission(output)
    archived, archive_tree, fixed, identity = source_profile(
        checkout, inventory, args.approved_linux, args.candidate,
    )
    candidate = identity.get('combined_commit', inventory['source_commit'])
    charon = (args.tool_root / 'aeneas/charon/bin/charon').resolve(strict=True)
    aeneas = (args.tool_root / 'aeneas/bin/aeneas').resolve(strict=True)
    lean_client = (args.tool_root / 'lean-mcp').resolve(strict=True)
    packages = args.packages.resolve(strict=True)
    require(digest(charon) == fixed['charon'], 'reviewed Charon binary differs')
    require(digest(aeneas) == fixed['aeneas'], 'reviewed Aeneas binary differs')
    require((lean_client / 'bin/lean-lsp-mcp').is_file(), 'original Lean service is missing')

    output.mkdir(mode=0o700)
    project = output / 'source'
    project.mkdir(mode=0o700)
    captures = output / 'artifacts'
    captures.mkdir(mode=0o700)
    report = dict(
        kind='hot-branch-close-local-development',
        commit=candidate,
        tree=archive_tree,
        target=fixed['rust_target'],
        status='failed',
        commands=[],
        **identity,
    )
    if admission:
        report['linux_admission'] = admission
    environment = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                       CARGO_INCREMENTAL='0', LEAN_NUM_THREADS='4')
    for variable in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        environment.pop(variable, None)

    def command(name, argv, *, artifact=None, extra_environment=None):
        effective_environment = environment | (extra_environment or {})
        log = artifact or (output / (name + '.log'))
        with log.open('xb') as stream:
            run = subprocess.run(argv, cwd=project, env=effective_environment,
                                 stdin=subprocess.DEVNULL, stdout=stream,
                                 stderr=subprocess.STDOUT, timeout=900, check=False)
        report['commands'].append(dict(
            name=name, argv=list(map(str, argv)), cwd=str(project), exit_code=run.returncode,
            output=str(log.relative_to(output)), log_sha256=digest(log),
        ))
        require(run.returncode == 0, f'{name} failed, see {log}')
        return log

    def capture(name, profile=None):
        ir = captures / (name + '.llbc')
        profile = inventory[name] if profile is None else profile
        selectors = [
            argument
            for flag, paths in profile.items()
            for item in paths
            for argument in (flag, item)
        ]
        command(name + '-charon', [
            charon, 'cargo', '--preset=aeneas', '--error-on-warnings',
            '--sysroot', 'default', *selectors, '--dest-file', ir, '--',
            '--manifest-path', 'controller-service/Cargo.toml', '--lib', '--locked',
            '--offline', '--target', fixed['rust_target'],
        ], extra_environment={'CARGO_TARGET_DIR': str(output / (name + '-target'))})
        raw = json.loads(ir.read_bytes())
        require(raw['has_errors'] is False, f'{name} emitted frontend errors')
        translated = raw['translated']
        options = translated['options']
        require(not any(options[key] for key in ('skip_borrowck', 'no_typecheck', 'monomorphize')),
                f'{name} borrow/type checking differs')
        require({row['key'] for row in translated['target_information']} == {fixed['rust_target']},
                f'{name} extraction target differs')
        require(all(options[flag[2:].replace('-', '_')] == paths
                    for flag, paths in profile.items()), f'{name} extraction scope differs')
        report[name + '_llbc_sha256'] = digest(ir)
        return ir

    try:
        archive = command('git-archive', ['git', '-C', checkout, 'archive', archived],
                          artifact=output / 'source.tar')
        with tarfile.open(archive) as source:
            source.extractall(project, filter='data')
        for name, expected in inventory['source_inputs'].items():
            require(digest(project / name) == expected, 'fixed source differs: ' + name)
        require((project / 'lean-toolchain').read_text().strip() == fixed['lean'],
                'fixed Lean toolchain differs')

        lean_entries = dict(inventory['lean_files'])
        generated_data = {}
        provider_files = {}
        provider_audits = []
        if args.with_blob_bridge:
            lean_entries.update(inventory['blob_bridge']['lean_files'])
        if args.with_window:
            window_ir = capture('window')
            window = command('window-owner-flow', [
                sys.executable, HERE / 'check_window_flow.py', window_ir, '--source-commit', candidate,
            ], artifact=output / 'window-owner-flow.json')
            report['window_owner_flow'] = json.loads(window.read_bytes())
            retire_ir = capture('retire')
            retire = command('retire-owner-flow', [
                sys.executable, HERE / 'check_retirement_flow.py', retire_ir,
            ], artifact=output / 'retire-owner-flow.json')
            report['retire_owner_flow'] = json.loads(retire.read_bytes())
            if args.with_load_key:
                profile = dict(inventory['retire'])
                profile['--include'] = [*profile['--include'],
                    'sandproof_controller_service::controller::branching::load']
                load_ir = capture('retire-load-key', profile)
                load = command('load-key-flow', [
                    sys.executable, HERE / 'check_load_key_flow.py', load_ir,
                ], artifact=output / 'load-key-flow.json')
                report['load_key_flow'] = json.loads(load.read_bytes())
            evidence = command('window-evidence', [
                sys.executable, HERE / 'window_evidence.py', window, retire,
                window_ir, retire_ir, candidate,
            ], artifact=captures / 'WindowEvidence.lean')
            generated_data[evidence.name] = evidence
            lean_entries[evidence.name] = dict(sha256=digest(evidence), generated=True, audits=[])
            lean_entries['CloseSql.lean'] = inventory['close_sql']
            if args.with_branch_scan:
                lean_entries['NormalBranchScan.lean'] = inventory['normal_branch_scan']

        (rust_relative, fixture_hash), = inventory['append_only'].items()
        fixture = HERE / 'roots.rs.in'
        require(digest(fixture) == fixture_hash, 'fixed extraction roots differ')
        rust = project / rust_relative
        original = rust.read_bytes()
        try:
            rust.write_bytes(original + fixture.read_bytes())
            batch_ir = capture('batch')
        finally:
            rust.write_bytes(original)
        require(digest(rust) == inventory['source_inputs'][rust_relative],
                'original Rust materialization changed')

        generated = captures / 'generated'
        command('batch-aeneas', [
            aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
            '-split-files', '-emit-json', '-namespace', LIB, '-subdir', LIB, '-dest',
            generated, batch_ir,
        ])
        package = project / VIEW / LIB
        package.mkdir(parents=True)
        lean_files = []
        for name, entry in lean_entries.items():
            source = generated_data.get(name)
            if source is None:
                source = (generated / LIB if entry['generated'] else HERE / 'lean') / name
            found = digest(source)
            require(found == entry['sha256'],
                    f"reviewed Lean input differs: {name}: expected {entry['sha256']}, got {found}")
            require(AUDIT.findall(source.read_text()) == entry['audits'], 'audit inventory differs: ' + name)
            shutil.copyfile(source, package / name)
            lean_files.append(str(VIEW / LIB / name))

        # Copy the existing reviewed V3 dependency view without changing it.
        current = project / CURRENT
        existing = json.loads((current / 'materialization.json').read_bytes())
        for entry in existing['modules'].values():
            source, destination = project / entry['source'], project / entry['path']
            require(digest(source) == entry['sha256'], 'original V3 provider differs: ' + entry['source'])
            if source != destination:
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, destination)
        (project / 'lakefile.toml').write_text(
            (current / 'lakefile.toml.in').read_text() +
            f'\n[[lean_lib]]\nname = "{LIB}"\nsrcDir = "{VIEW}"\n'
        )
        if args.with_blob_bridge:
            blob_inventory = project / blob_provider.INVENTORY
            require(digest(blob_inventory) == inventory['blob_bridge']['provider_inventory_sha256'],
                    'the original two-BLOB provider inventory changed')
            blob_view = json.loads(blob_inventory.read_bytes())['current_view']
            installer = blob_provider.stage(archive, blob_inventory, output / 'blob-provider-original')
            install_log = command('original-blob-provider', [
                sys.executable, '-B', installer, '--project', project,
            ])
            installed = json.loads(install_log.read_bytes())
            require(installed['inventory_sha256'] == digest(blob_inventory),
                    'materialized a different two-BLOB provider')
            provider_files = blob_view['files']
            require(all(installed['changed'].get(name) == expected and
                        digest(project / name) == expected for name, expected in provider_files.items()),
                    'the original two-BLOB proof or provider changed')
            provider_audits = blob_view['audits']
            require(sorted(provider_audits) == sorted(
                audit for name in provider_files for audit in AUDIT.findall((project / name).read_text())
            ), 'the original two-BLOB audit inventory changed')
            lean_files.extend(provider_files)
        (project / '.lake').mkdir()
        (project / '.lake/packages').symlink_to(packages, target_is_directory=True)
        (project / '.tools').mkdir()
        (project / '.tools/lean-mcp').symlink_to(lean_client, target_is_directory=True)
        artifacts = project / 'artifacts'
        artifacts.mkdir()

        lean_version = command('lean-version', ['lean', '--version'])
        require(f'(version {fixed["lean"].rsplit("v", 1)[-1]},' in lean_version.read_text(),
                'actual Lean version differs')
        targets = [LIB + '.CloseProof']
        if args.with_window:
            targets.append(LIB + '.CloseSql')
        if args.with_branch_scan:
            targets.append(LIB + '.NormalBranchScan')
        if args.with_blob_bridge:
            targets.extend((LIB + '.CloseSqlBlobProducer', LIB + '.CloseSqlBlobConsumer'))
        command('lake-build', ['lake', 'build', *targets])
        for index, relative in enumerate(lean_files):
            label = 'kernel-' + str(index)
            command(label, ['lake', 'env', 'lean', relative], artifact=artifacts / (label + '.log'))
        command('actual-file-service', [
            lean_client / 'bin/python', '-B', '-c',
            'import sys; from tools import proof_services; '
            'proof_services.FILES=tuple(sys.argv[1:]); proof_services.main()', *lean_files,
        ])
        for name, entry in lean_entries.items():
            require(digest(package / name) == entry['sha256'], 'verified Lean bytes changed: ' + name)
        require(all(digest(project / name) == expected for name, expected in provider_files.items()),
                'the verified two-BLOB provider bytes changed')
        (service_output,) = artifacts.glob('physical-service-*')
        physical = service_output.rename(artifacts / 'physical-files')
        actual = dict(
            complete=True,
            files={str(VIEW / LIB / name): entry['sha256'] for name, entry in lean_entries.items()}
                | provider_files,
            audits=[theorem for entry in lean_entries.values() for theorem in entry['audits']]
                + provider_audits,
            service_log_sha256=digest(physical / 'service.log'),
        )
        # Check the actual saved artifacts with the original Stage1 checker.
        # This does not issue a native or formal acceptance seal.
        sys.path.insert(0, str(project))
        spec = importlib.util.spec_from_file_location(
            'stage1_branch_actual_registry', project / 'verification/experiments/stage1-protocol/registered.py',
        )
        registry = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(registry)
        registry._saved_physical_view(actual, project, physical, report['commands'])
        report.update(status='passed', lean_files=lean_files,
                      audit_count=len(actual['audits']), saved_actual_view=actual)
        if args.approved_linux:
            result = output / 'result.json'
            result.write_text(json.dumps(report, indent=2) + '\n')
            try:
                evidence = registry._checkpoint_evidence(
                    registry._execution_helper(project),
                    dict(name='hot-branch-close-linux-development', actual_files=True),
                    report, result, output / 'lake-build.log',
                )
                expected_captures = ['window', 'retire', 'batch']
                if args.with_load_key:
                    expected_captures.append('retire-load-key')
                require(all(str(captures / (name + '.llbc')) in evidence for name in expected_captures),
                        'the original evidence reader omitted a current extraction')
            except Exception:
                report['status'] = 'failed'
                raise
            checkpoint = dict(
                kind='hot-branch-close-linux-checkpoint-readback',
                candidate=candidate, tree=archive_tree, files=len(evidence),
                result_sha256=digest(result),
            )
            (output / 'checkpoint-readback.json').write_text(json.dumps(checkpoint, indent=2) + '\n')
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f'{report["audit_count"]} local audit results and {len(lean_files)} actual Lean files: {output}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checkout', type=Path, default=ROOT)
    parser.add_argument('--tool-root', type=Path, required=True)
    parser.add_argument('--packages', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--with-window', action='store_true')
    parser.add_argument('--with-blob-bridge', action='store_true')
    parser.add_argument('--with-branch-scan', action='store_true')
    parser.add_argument('--with-load-key', action='store_true')
    parser.add_argument('--approved-linux', action='store_true')
    parser.add_argument('--candidate', help='the pinned combined Linux commit')
    replay(parser.parse_args())
