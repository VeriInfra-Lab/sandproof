"""G1/G2 native examples or original extraction preflight, never a proof."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
DIRECTORY = Path('verification/experiments/plain-field-carriers')
PACKAGE = 'sandproof-plain-field-carriers-extract'
CRATE = 'sandproof_plain_field_carriers_extract'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path):
    return json.loads(path.read_text())


def source_inputs(root):
    values = {}
    for line in (root / DIRECTORY / 'inputs.sha256').read_text().splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  (.+)', line)
        if not match or match[2] in values:
            raise ValueError('malformed or duplicate preflight input')
        name = Path(match[2])
        if name.is_absolute() or '..' in name.parts:
            raise ValueError('escaping source input')
        path = root / name
        if path.is_symlink() or not path.is_file() or sha(path) != match[1]:
            raise ValueError('source input changed: ' + str(name))
        values[str(name)] = match[1]
    profile = read(root / DIRECTORY / 'source-inputs.json')
    if (profile['roots'] != ['tenant_new', 'tenant_value', 'sandbox_new', 'sandbox_value', 'versions_current']
            or not set(profile['production_sources']) <= set(values)):
        raise ValueError('required real sources/roots omitted')
    for name, digest in profile['production_sources'].items():
        if values[name] != digest:
            raise ValueError('production source differs from original baseline')
    required = {str(DIRECTORY / n) for n in ('Cargo.toml', 'Cargo.lock', 'src/lib.rs',
                'tests/carriers.rs', 'test_inspection.py', 'run.py', 'runtime.sh',
                'README.md', 'source-inputs.json')}
    if not required <= set(values):
        raise ValueError('incomplete own preflight source closure')
    return values, profile


def command(args, root, output, name, record, timeout=600):
    log = output / (name + '.log')
    start = time.monotonic()
    with log.open('xb') as stream:
        code = subprocess.run(args, cwd=root, stdout=stream, stderr=subprocess.STDOUT,
                              timeout=timeout).returncode
    record['steps'].append(dict(name=name, command=args, exit_code=code,
                                seconds=time.monotonic() - start, log_sha256=sha(log)))
    if code:
        raise ValueError(name + ' failed; retain exact phase/log, not a semantic counterexample')
    return log


def native(root, output, record):
    expected = read(root / 'verification/foundations-environment.json')['production_compiler']
    actual = subprocess.check_output(['rustc', '--version'], text=True).strip()
    if actual != expected:
        raise ValueError('ordinary compiler differs from fixed production compiler')
    record['production_rustc'] = subprocess.check_output(['rustc', '-vV'], text=True)
    manifest = str(DIRECTORY / 'Cargo.toml')
    metadata_log = command(['cargo', 'metadata', '--manifest-path', manifest, '--locked', '--offline',
                            '--format-version=1'], root, output, 'metadata', record)
    metadata_rows = [json.loads(line) for line in metadata_log.read_text().splitlines()
                     if line.startswith('{') and '"packages"' in line]
    if len(metadata_rows) != 1:
        raise ValueError('one complete actual metadata record required')
    own = {'sandbox-controller-core': ('controller-core/Cargo.toml', 'controller-core/src/lib.rs'),
           'sandproof-protocol': ('sandbox-protocol/Cargo.toml', 'sandbox-protocol/src/lib.rs'),
           PACKAGE: (manifest, str(DIRECTORY / 'src/lib.rs'))}
    packages = {p['id']: p for p in metadata_rows[0]['packages'] if p['name'] in own}
    if {p['name'] for p in packages.values()} != set(own):
        raise ValueError('three actual own packages required')
    build = command(['cargo', 'test', '--manifest-path', manifest, '--locked', '--offline',
                     '--no-run', '--tests', '--message-format=json'], root, output, 'build', record)
    messages = []
    for line in build.read_text().splitlines():
        if line.startswith('{'):
            item = json.loads(line)
            if item.get('reason') == 'compiler-artifact':
                messages.append(item)
    libraries, test = {}, []
    for item in messages:
        package = packages.get(item['package_id'])
        if not package:
            continue
        name = package['name']
        if Path(item['manifest_path']).resolve() != (root / own[name][0]).resolve():
            raise ValueError('actual package manifest is not this original source')
        if 'lib' in item['target']['kind'] and item['profile']['test'] is False:
            if (item['fresh'] is not False or name in libraries
                    or Path(item['target']['src_path']).resolve() != (root / own[name][1]).resolve()):
                raise ValueError('cached or wrong source library cannot satisfy first preflight')
            libraries[name] = item
        if name == PACKAGE and item['target']['kind'] == ['test'] and item['target']['name'] == 'carriers':
            if (item['fresh'] is not False or Path(item['target']['src_path']).resolve()
                    != (root / DIRECTORY / 'tests/carriers.rs').resolve()):
                raise ValueError('wrong test source or cached test executable')
            test.append(item)
    if set(libraries) != set(own) or len(test) != 1:
        raise ValueError('missing/duplicate own libraries or actual test binary')
    executable = Path(test[0]['executable'])
    result = command([str(executable), '--test-threads=1'], root, output, 'boundary-examples', record)
    if not re.search(r'test result: ok\. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;', result.read_text()):
        raise ValueError('missing/filtered native boundary examples')
    record.update(libraries=libraries, test_binary=test[0], test_binary_sha256=sha(executable))
    command(['cargo', 'fmt', '--manifest-path', manifest, '--check'], root, output, 'fmt', record)
    command(['cargo', 'clippy', '--manifest-path', manifest, '--tests', '--locked', '--offline',
             '--', '-D', 'warnings'], root, output, 'clippy', record)


def tools(root, tool_root):
    environment = read(root / 'verification/foundations-environment.json')
    for name, repository, binary in [('charon', 'aeneas/charon', 'aeneas/charon/bin/charon'),
                                     ('aeneas', 'aeneas', 'aeneas/bin/aeneas')]:
        revision = subprocess.check_output(['git', '-C', str(tool_root / repository), 'rev-parse', 'HEAD'], text=True).strip()
        dirty = subprocess.check_output(['git', '-C', str(tool_root / repository), 'status',
                                        '--porcelain', '--untracked-files=no'], text=True)
        if revision != environment[name]['revision'] or dirty or sha(tool_root / binary) != environment[name]['binary_sha256']:
            raise ValueError('fixed original tool source/binary changed: ' + name)
    charon = tool_root / 'aeneas/charon/bin/charon'
    if subprocess.check_output([str(charon), 'toolchain-version'], text=True).strip() != environment['extraction_frontend']:
        raise ValueError('original extraction compiler differs')
    sysroot = Path(subprocess.check_output([str(charon), 'toolchain-path'], text=True).strip())
    if subprocess.check_output([str(sysroot / 'bin/rustc'), '--version'], text=True).strip() != environment['extraction_compiler']:
        raise ValueError('actual frontend binary version differs')
    for name, digest in environment['library_sources']['extraction'].items():
        if sha(sysroot / 'lib/rustlib/src/rust/library' / name) != digest:
            raise ValueError('fixed standard-library input changed: ' + name)
    return environment


def extract(root, output, record, profile, tool_root):
    if str(root) != '/tmp/plain-field-carriers/source' or Path.cwd() != root:
        raise ValueError('use the registered private source mapping/cwd')
    for name, expected in [('CARGO_HOME', '/tmp/plain-field-carriers/cargo-home'),
                           ('CARGO_TARGET_DIR', '/tmp/plain-field-carriers/target-formal'),
                           ('TMPDIR', '/tmp/plain-field-carriers/tmp')]:
        if os.environ.get(name) != expected:
            raise ValueError('unregistered mapping: ' + name)
    for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'LD_PRELOAD', 'LD_LIBRARY_PATH'):
        if os.environ.get(name):
            raise ValueError('unregistered compiler/runtime flags')
    record['fixed_environment'] = tools(root, tool_root)
    llbc = output / 'original.llbc'
    args = [str(tool_root / 'aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default', '--include', 'sandbox_controller_core',
            '--include', 'sandproof_protocol']
    for name in profile['roots']:
        args += ['--start-from', 'crate::' + name]
    args += ['--dest-file', str(llbc), '--', '--manifest-path', str(DIRECTORY / 'Cargo.toml'),
             '--lib', '--locked', '--offline', '--target', 'x86_64-unknown-linux-gnu']
    command(args, root, output, 'charon', record)
    ir = read(llbc)
    if ir.get('has_errors') is not False:
        raise ValueError('original frontend reported errors/incomplete IR')
    options = ir['translated']['options']
    if (any(options[name] for name in ('skip_borrowck', 'no_typecheck', 'no_normalize', 'monomorphize'))
            or options['opaque'] or options['exclude']):
        raise ValueError('unreviewed extraction bypass, opaque or exclusion')
    generated = output / 'generated'
    generated.mkdir()
    command([str(tool_root / 'aeneas/bin/aeneas'), '-backend', 'lean', '-abort-on-error',
             '-warnings-as-errors', '-split-files', '-emit-json', '-namespace', 'SandproofPlainFieldCarriersRust',
             '-subdir', 'SandproofPlainFieldCarriers', '-dest', str(generated), str(llbc)],
            root, output, 'aeneas', record)
    inspect_generated(output, profile, record)


def generated_signature(types_text, funs_text):
    """Recognize these fixed original signatures; this is not a Lean parser/proof."""
    version_name = 'sandproof_protocol.internal.ContractVersions'
    match = re.search(r'^structure ' + re.escape(version_name) + r' where\n((?:  [^\n]+\n)+)',
                      types_text, re.MULTILINE)
    expected = [('internal_protocol', 'Std.U16'), ('center_store', 'Std.U32'),
                ('host_store', 'Std.I64')]
    if not match or [line.strip() for line in match[1].splitlines()] != [
            name + ' : ' + typ for name, typ in expected]:
        raise ValueError('actual version fields must be exactly Std.U16/Std.U32/Std.I64')
    constants = [('INTERNAL_PROTOCOL_VERSION', 'U16', '2', 'u16'),
                 ('CENTER_STORE_FORMAT_VERSION', 'U32', '6', 'u32'),
                 ('HOST_STORE_FORMAT_VERSION', 'I64', '16', 'i64')]
    for name, typ, value, suffix in constants:
        line = 'def sandproof_protocol.internal.' + name + ' : Std.' + typ + ' := ' + value + '#' + suffix
        if funs_text.splitlines().count(line) != 1:
            raise ValueError('original typed version constant changed: ' + name)
    aliases = {}
    for name in ('TenantId', 'SandboxId'):
        line = 'def sandbox_controller_core.ids.' + name + ' := Std.U64'
        annotation = '@[reducible, rust_type "sandbox_controller_core::ids::' + name + '"]\n'
        if types_text.splitlines().count(line) != 1 or annotation + line not in types_text:
            raise ValueError('original scalar carrier representation changed: ' + name)
        aliases[name] = 'reducible Std.U64 alias; Rust nominal distinction is not Lean nominal isolation'
    return dict(version_fields=expected, typed_constants=constants, id_representation=aliases)


def inspect_generated(output, profile, record):
    llbc = output / 'original.llbc'
    ir = read(llbc)
    if ir.get('has_errors') is not False:
        raise ValueError('original frontend reported errors/incomplete IR')
    options = ir['translated']['options']
    if (any(options[name] for name in ('skip_borrowck', 'no_typecheck', 'no_normalize', 'monomorphize'))
            or options['opaque'] or options['exclude']
            or options['start_from'] != ['crate::' + name for name in profile['roots']]
            or options['include'] != ['sandbox_controller_core', 'sandproof_protocol']):
        raise ValueError('unreviewed extraction bypass, roots, opaque or exclusion')
    generated = output / 'generated'
    inventory = read(generated / 'translation.json')
    selectors = [(name, [CRATE + '::' + name]) for name in profile['roots']]
    selectors += [(kind + '-' + method, ['sandbox_controller_core::ids::' + kind + '}', '::' + method])
                  for kind in ('TenantId', 'SandboxId') for method in ('new', 'value')]
    selectors += [('actual-versions-current', ['sandproof_protocol::internal::ContractVersions}', '::current'])]
    declarations = {f['def_id']: f for f in ir['translated']['fun_decls'] if f}
    actual = {}
    for key, tokens in selectors:
        found = [f for f in inventory['functions'] if all(token in f['rust_name'] for token in tokens)]
        if len(found) != 1 or found[0]['is_opaque'] or not isinstance(declarations[found[0]['def_id']].get('body'), dict):
            raise ValueError('required original/delegated body missing or opaque: ' + key)
        if 'Structured' not in declarations[found[0]['def_id']]['body']:
            raise ValueError('required original body is not structured: ' + key)
        actual[key] = found[0]
    type_rows = []
    for suffix in ('sandbox_controller_core::ids::TenantId', 'sandbox_controller_core::ids::SandboxId',
                   'sandproof_protocol::internal::ContractVersions'):
        found = [t for t in inventory['types'] if t['rust_name'] == suffix]
        if len(found) != 1 or 'External' in found[0]['lean_file']:
            raise ValueError('required real carrier type not emitted transparently: ' + suffix)
        type_rows += found
    signature = generated_signature(
        (generated / 'SandproofPlainFieldCarriers/Types.lean').read_text(),
        (generated / 'SandproofPlainFieldCarriers/Funs.lean').read_text())
    if len(inventory['functions']) != 10 or len(inventory['types']) != 3:
        raise ValueError('unexpected generated function/type set requires review')
    record.update(original_and_root_functions=actual, actual_types=type_rows,
        generated_signatures=signature,
        options=options, llbc_sha256=sha(llbc),
        opaque_functions=[f for f in inventory['functions'] if f['is_opaque']],
        drop_glue=[f for f in ir['translated']['fun_decls'] if f and 'DropGlue' in json.dumps(f['item_meta']['name'])],
        literal_drop_tokens=llbc.read_text().count('"Drop"'),
        dictionary_sizes={key: dict(slots=len(ir['translated'][key]),
                                    present=sum(v is not None for v in ir['translated'][key]))
                          for key in ('type_decls', 'fun_decls', 'global_decls', 'trait_decls', 'trait_impls')},
        generated_templates=[str(p.relative_to(generated)) for p in generated.rglob('*')
                             if p.is_file() and ('External' in p.name or 'Template' in p.name)],
        generated_sha256={str(p.relative_to(generated)): sha(p) for p in generated.rglob('*') if p.is_file()},
        scope='original transparent signature/Lean generation preflight; no kernel/model/theorem/full-suite acceptance')


def review(root, original, profile, record):
    """Post-check one preserved failed receipt; never rerun or rewrite either tool."""
    previous = read(original / 'result.json')
    expected_error = "ValueError('actual signed host_store representation requires review; never substitute Nat')"
    if (previous.get('kind') != 'plain-field-carrier-preflight-not-proof'
            or previous.get('phase') != 'extract' or previous.get('success') is not False
            or previous.get('error') != expected_error):
        raise ValueError('expected the preserved original post-inspection failure')
    current = source_inputs(root)[0]
    changed_control = {str(DIRECTORY / name) for name in ('run.py', 'README.md')}
    added_control = {str(DIRECTORY / 'test_inspection.py')}
    old_sources = previous['source_sha256']
    if (set(current) != set(old_sources) | added_control
            or any(current[name] != digest for name, digest in old_sources.items()
                   if name not in changed_control)):
        raise ValueError('original compilation/extraction source closure changed')
    if previous['fixed_environment'] != read(root / 'verification/foundations-environment.json'):
        raise ValueError('fixed original tool/library configuration changed')
    steps = previous['steps']
    if [step['name'] for step in steps] != ['charon', 'aeneas'] or any(step['exit_code'] for step in steps):
        raise ValueError('both actual original tool exits must be zero')
    for step in steps:
        if sha(original / (step['name'] + '.log')) != step['log_sha256']:
            raise ValueError('original actual tool log changed')
    inspect_generated(original, profile, record)
    record.update(kind='plain-field-carrier-original-output-review-not-extraction-or-proof',
        original_receipt_sha256=sha(original / 'result.json'),
        original_success=False, original_error=previous['error'], reused_original_tool_steps=steps,
        control_source_changes={name: dict(previous=old_sources[name], current=current[name])
                                for name in sorted(changed_control)},
        note='Only this post-inspection ran; no compiler/Charon/Aeneas/Lean invocation. Original failure remains unchanged.')


def run(root, output, phase, tool_root, original=None):
    if output.exists() or output.is_symlink():
        raise ValueError('fresh evidence directory required; preserve prior attempts')
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='plain-field-carrier-preflight-not-proof', phase=phase, steps=[])
    try:
        before, profile = source_inputs(root)
        record['source_sha256'] = before
        if phase == 'native':
            native(root, output, record)
        elif phase == 'review':
            review(root, original, profile, record)
        else:
            extract(root, output, record, profile, tool_root)
        if source_inputs(root)[0] != before:
            raise ValueError('source changed during actual preflight')
        record['success'] = True
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic() - start
    (output / 'result.json').write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=['native', 'extract', 'review'])
    parser.add_argument('--root', type=Path, default=ROOT)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tool-root', type=Path)
    parser.add_argument('--original', type=Path, help='preserved extract-first evidence for review only')
    args = parser.parse_args()
    if args.phase == 'review' and args.original is None:
        parser.error('review requires --original; it never runs extraction')
    result = run(args.root.resolve(strict=True), args.output.absolute(), args.phase, args.tool_root, args.original)
    print(json.dumps({k: v for k, v in result.items() if k in ('kind', 'phase', 'success', 'seconds', 'error')}, ensure_ascii=False))
    raise SystemExit(0 if result['success'] else 1)
