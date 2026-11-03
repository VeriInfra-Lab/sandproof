"""Fixed original compiler captures and independent named SHA views."""
from pathlib import Path
import json

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-save-digest')
METHOD = 'sandproof_controller_service::controller::lifecycle_inbox::codec::_::digest'
RECEIVER = 'sandproof_controller_service::controller::checkpoint::config::StandardConfigRead'
SAVE_RECEIVER='sandproof_controller_service::controller::lifecycle_inbox::codec::StandardInboxSave'
SAVE_THIN='sandproof_controller_service::controller::lifecycle_inbox::codec::proof_stage1_save_digest'
READ_THIN='sandproof_controller_service::controller::lifecycle_inbox::codec::proof_stage1_read_save_digest'
LEAF = 'sandproof_controller_service::controller::lifecycle_inbox::sha256::digest'


def run(project, folder, label, support, command, kind='named'):
    original_capture = support.module('standard_digest_original_capture_wrapper',
        ROOT/HERE.parent/'standard-blob/capture.py')
    binding = support.module('standard_digest_fixed_interface_binding', ROOT/HERE/'binding.py')
    if kind == 'named':
        roots = ['crate::controller::lifecycle_inbox::codec::proof_stage1_save_digest', 'crate::controller::lifecycle_inbox::codec::proof_stage1_read_save_digest']
        include, opaque = [METHOD, SAVE_RECEIVER, RECEIVER, SAVE_THIN, READ_THIN], [LEAF]
    elif kind in ('original', 'leaf'):
        # Both observations bind the one adopted two-call library leaf.
        # Caller bodies are independently transparent in the named capture.
        roots = ['crate::controller::lifecycle_inbox::sha256::digest']
        include, opaque = [LEAF], []
    else:
        raise ValueError('unknown original SHA capture view')
    fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
    for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
        if support.sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
            raise ValueError('fixed binary changed: '+name)
    virtual = support.runtime.VIRTUAL
    ir = project/'artifacts'/('named-leaf-full.llbc' if kind == 'leaf' else
        'standard-save-digest.llbc' if kind == 'named' else 'direct-save-digest.llbc')
    argv = [str(virtual/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default']
    for flag, values in [('--include', include), ('--opaque', opaque), ('--start-from', roots)]:
        for value in values:
            argv += [flag, value]
    argv += ['--dest-file', str(virtual/'artifacts'/ir.name), '--', '--manifest-path',
             'controller-service/Cargo.toml', '--lib', '--locked', '--offline',
             '--target', fixed['target']]
    command(project, folder, label+'-charon', original_capture.wrapper(project, support)+argv)
    if not ir.is_file() or ir.is_symlink() or ir.stat().st_size > 128*2**20:
        raise ValueError('bounded original IR parser input refused')
    raw = json.loads(ir.read_text())
    options = raw['translated']['options']
    if (raw['has_errors'] or options['start_from'] != roots or options['include'] != include
            or options['opaque'] != opaque or options['exclude'] or options['skip_borrowck']
            or options['no_typecheck'] or options['monomorphize'] or options['remove_unused_clauses']):
        raise ValueError('incomplete or weakened original compiler view')
    if {v['key'] for v in raw['translated']['target_information']} != {fixed['target']}:
        raise ValueError('wrong fixed compiler target')
    row = dict(kind=kind, roots=roots, include=include, opaque=opaque, ir=str(ir),
        ir_sha256=support.sha(ir), ir_value_sha256=support.ir_value_digest(raw),
        original_IR_bytes=ir.stat().st_size, generated_bodies_modified=False,
        business_function_opacity_added=False)
    if kind != 'named':
        row['source_binding'] = binding.admit(raw)
        row['original_direct_method_machine_refined'] = False
        return row
    generated = folder/('generated-'+label)
    generated.mkdir()
    command(project, folder, label+'-aeneas', [str(ROOT/'.tools/aeneas/bin/aeneas'),
        '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
        '-emit-json', '-namespace', 'SandproofStandardSaveDigestRust', '-subdir',
        'SandproofStandardSaveDigest', '-dest', str(generated), str(ir)])
    translation = json.loads((generated/'translation.json').read_text())
    functions = [{k: f[k] for k in ('lean_name', 'rust_name', 'is_opaque', 'lean_file')}
                 for f in translation['functions']]
    expected = json.loads((ROOT/HERE/'function-inventory.json').read_text())['functions']
    if functions != expected:
        raise ValueError('actual method/thin root/library boundary function census differs')
    row.update(generated_directory=str(generated), translation=translation,
        generated_files={str(p.relative_to(generated)): support.sha(p)
            for p in generated.rglob('*') if p.is_file()})
    return row


def generic_save(project,folder,label,support,command):
    """Fresh unchanged generic Save/validation roots, with the existing
    explicit canonical ContractError view or primitive Graphic view."""
    admission=support.module('standard_save_original_generic_admission',
        ROOT/HERE.parent/'inbox-save/admission.py')
    primitive=label=='primitive'
    if label not in ('original','canonical','primitive'):
        raise ValueError('unknown generic Save source view')
    roots=admission.PRIMITIVE_ROOTS if primitive else admission.ROOTS
    include=admission.PRIMITIVE_INCLUDE if primitive else admission.INCLUDE
    opaque=['rusqlite::error::Error']+(['sandproof_protocol::ContractError'] if label=='canonical' else [])
    wrapper=support.module('save_generic_fixed_wrapper',ROOT/HERE.parent/'standard-blob/capture.py')
    fixed=json.loads((ROOT/'verification/foundations-environment.json').read_text())
    for name,suffix in [('charon','charon/bin/charon'),('aeneas','bin/aeneas')]:
        if support.sha(ROOT/'.tools/aeneas'/suffix)!=fixed[name]['binary_sha256']:
            raise ValueError('fixed compiler/extractor changed')
    virtual=support.runtime.VIRTUAL
    ir=project/'artifacts'/('save-'+label+'.llbc')
    args=[str(virtual/'.tools/aeneas/charon/bin/charon'),'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default']
    for flag,values in [('--include',include),('--opaque',opaque),('--start-from',roots)]:
        for value in values:args += [flag,value]
    args += ['--dest-file',str(virtual/'artifacts'/ir.name),'--','--manifest-path',
        'verification/experiments/stage1-protocol/inbox-record/Cargo.toml','--lib','--locked','--offline','--target',fixed['target']]
    command(project,folder,label+'-charon',wrapper.wrapper(project,support)+args)
    if ir.is_symlink() or not ir.is_file() or ir.stat().st_size>128*2**20:
        raise ValueError('bounded original generic Save IR refused')
    raw=json.loads(ir.read_text());options=raw['translated']['options']
    if raw['has_errors'] or options['include']!=include or options['start_from']!=roots or options['opaque']!=opaque or options['exclude'] or options['skip_borrowck'] or options['no_typecheck'] or options['monomorphize'] or options['remove_unused_clauses']:
        raise ValueError('incomplete or weakened original generic Save capture')
    if {r['key'] for r in raw['translated']['target_information']}!={fixed['target']}:
        raise ValueError('wrong target')
    admission.OPAQUE=opaque
    binding=admission.admit_ir(raw,opaque=opaque,primitive=primitive)
    generated=folder/('generated-'+label);generated.mkdir()
    namespace='SandproofStage1Graphic' if primitive else 'SandproofStage1Request'
    command(project,folder,label+'-aeneas',[str(ROOT/'.tools/aeneas/bin/aeneas'),'-backend','lean',
        '-abort-on-error','-warnings-as-errors','-split-files','-emit-json','-namespace',namespace+'Rust',
        '-subdir',namespace,'-dest',str(generated),str(ir)])
    translation=admission.admit_generated(generated,'primitive' if primitive else 'correct')
    return dict(kind='generic-save-'+label,roots=roots,include=include,opaque=opaque,
        ir=str(ir),ir_sha256=support.sha(ir),ir_value_sha256=support.ir_value_digest(raw),
        actual_root_binding=binding,translation=translation,generated_directory=str(generated),
        generated_files={str(p.relative_to(generated)):support.sha(p)for p in generated.rglob('*')if p.is_file()},
        actual_save_body_modified=False,generated_bodies_modified=False)
