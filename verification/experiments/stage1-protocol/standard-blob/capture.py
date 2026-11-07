"""Fixed compiler/extractor views; no generated body is edited or synthesized."""
from pathlib import Path
import json
import os

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-blob')


def wrapper(project, support):
    virtual = support.runtime.VIRTUAL
    paths = [Path.home(), support.runtime.tool_root(ROOT), support.runtime.package_root(ROOT)]
    for name in ('CARGO_HOME', 'RUSTUP_HOME', 'ELAN_HOME', 'SANDPROOF_PROOF_RUNTIME_ROOT'):
        if os.environ.get(name):
            paths.append(Path(os.environ[name]).resolve(strict=True))
    return ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
        '--tmpfs', '/tmp', '--tmpfs', '/home',
        *[part for path in dict.fromkeys(paths) for part in ('--ro-bind', str(path), str(path))],
        '--bind', str(project), str(virtual), '--chdir', str(virtual),
        '--setenv', 'CARGO_TARGET_DIR', str(virtual/'artifacts/extract-target'),
        '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
        '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']


def run(project, folder, label, support, command, kind):
    admission = support.module('blob_existing_load_root_admission_'+label,
        ROOT/HERE.parent/'inbox-load/admission.py')
    if kind == 'standard':
        roots = ['crate::controller::lifecycle_inbox::codec::proof_stage1_load_blob_pair']
        include = ['sandproof_controller_service', 'sandbox_controller_core', 'sandproof_protocol',
                   'rusqlite::params::*::__bind_in']
        opaque = ['rusqlite::error::Error', 'rusqlite::types::from_sql::FromSqlError',
                  'rusqlite::types::value_ref::ValueRef']
        manifest = 'controller-service/Cargo.toml'
        namespace = 'SandproofLoadBlob'
    elif kind in ('load-original', 'load-canonical', 'primitive'):
        primitive = kind == 'primitive'
        roots = admission.PRIMITIVE_ROOTS if primitive else admission.ROOTS
        include = admission.PRIMITIVE_INCLUDE if primitive else admission.INCLUDE
        opaque = ['rusqlite::error::Error']
        if kind == 'load-canonical':
            opaque += ['sandproof_protocol::ContractError']
        manifest = 'verification/experiments/stage1-protocol/inbox-record/Cargo.toml'
        namespace = 'SandproofStage1Graphic' if primitive else 'SandproofStage1Request'
    else:
        raise ValueError('unknown fixed view kind')
    fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
    for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
        if support.sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
            raise ValueError('fixed proof binary differs: '+name)
    virtual = support.runtime.VIRTUAL
    ir = project/'artifacts'/(label+'.llbc')
    argv = [str(virtual/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default']
    for option, values in [('--include', include), ('--opaque', opaque), ('--start-from', roots)]:
        for value in values:
            argv += [option, value]
    argv += ['--dest-file', str(virtual/'artifacts'/ir.name), '--', '--manifest-path', manifest,
             '--lib', '--locked', '--offline', '--target', fixed['target']]
    command(project, folder, label+'-charon', wrapper(project, support)+argv)
    if not ir.is_file() or ir.is_symlink() or ir.stat().st_size > 128*2**20:
        raise ValueError('original IR exceeded the reviewed bounded parser input')
    raw = json.loads(ir.read_text())
    options = raw['translated']['options']
    if (raw['has_errors'] or options['start_from'] != roots or options['include'] != include
            or options['opaque'] != opaque or options['exclude'] or options['skip_borrowck']
            or options['no_typecheck'] or options['monomorphize'] or options['remove_unused_clauses']):
        raise ValueError('unreviewed or incomplete real compiler capture')
    if {t['key'] for t in raw['translated']['target_information']} != {fixed['target']}:
        raise ValueError('wrong fixed target')
    binding = None
    if kind != 'standard':
        # Reuse the exact original root/callee checks; the extra canonical
        # data-type alias is the already-adopted FullCreate binding.
        admission.OPAQUE = list(opaque)
        binding = admission.admit_ir(raw, opaque=opaque, primitive=kind == 'primitive')
    target = folder/('generated-'+label)
    target.mkdir()
    command(project, folder, label+'-aeneas', [str(ROOT/'.tools/aeneas/bin/aeneas'),
        '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
        '-namespace', namespace+'Rust', '-subdir', namespace, '-dest', str(target), str(ir)])
    translated = json.loads((target/'translation.json').read_text())
    if kind == 'standard':
        expected = json.loads((ROOT/HERE/'function-inventory.json').read_text())
        observed = [{key: f[key] for key in ('lean_name', 'rust_name', 'is_opaque', 'lean_file')}
                    for f in translated['functions']]
        if observed != expected['functions']:
            raise ValueError('standard actual six transparent/five library function inventory differs')
    else:
        admission.admit_generated(target, 'primitive' if kind == 'primitive' else 'correct')
    generated = {str(p.relative_to(target)): support.sha(p)
                 for p in target.rglob('*') if p.is_file()}
    return dict(kind=kind, roots=roots, include=include, opaque=opaque,
        ir_sha256=support.sha(ir), ir_value_sha256=support.ir_value_digest(raw),
        original_IR_bytes=ir.stat().st_size,
        ir=str(ir), generated_directory=str(target), generated_files=generated,
        translation=translated, original_root_binding=binding,
        generated_bodies_edited=False, business_function_opacity_added=False)
