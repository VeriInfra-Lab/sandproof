"""Materialize the checked representation bridge for the two actual revision views."""
import argparse
import json
from pathlib import Path
import subprocess

from prepare import HERE, ROOT, STAGE, apply_view, checked, sha


BASE = 'SandproofUnloadBaseSnapshot'
PARENT = 'SandproofUnloadFirstObservation'
LINK = 'RevisionReadyTransport'
PIN = '505b6ca35217e7be5c96c3e2f8045edfbdf47291'
TEMPLATE = HERE / 'revision-transport'
BASE_PROOF = STAGE / 'unload-base-provenance/proof-inputs'
PARENT_PROOF = STAGE / 'unload-startup-source/proof-inputs'
SQL_CARRIERS = {
    'rusqlite.row.Row': 'Row',
    'rusqlite.statement.Statement': 'Statement',
    'rusqlite.types.to_sql.ToSqlOutput': 'ToSqlOutput',
    'rusqlite.types.value_ref.ValueRef': 'ValueRef',
    'rusqlite.types.from_sql.FromSqlError': 'FromSqlError',
}
SQL_FIELDS = {key.replace('.', '_'): value for key, value in SQL_CARRIERS.items()}
TOOLS = ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'tools/proof_services.py',
         'scripts/lean-mcp-server.sh')


def replace_once(text, before, after):
    if text.count(before) != 1:
        raise ValueError('historical adapter rewrite has no unique source')
    return text.replace(before, after, 1)


def base_types(legacy):
    fields = ''.join(f'  {field} : Type\n' for field in SQL_FIELDS)
    fields += '  sandproof_protocol_SandboxId : Type\n'
    methods = ''.join(
        f'abbrev {name} [r : Representations] := r.{name.replace(".", "_")}\n'
        for name in SQL_CARRIERS
    )
    methods += 'abbrev sandproof_protocol.SandboxId [r : Representations] := r.sandproof_protocol_SandboxId\n\n'
    header = 'class Representations where\n'
    legacy = replace_once(legacy, header, header + fields)
    return replace_once(legacy, f'end {BASE}', methods + f'end {BASE}')


def transport(legacy, theorems):
    # Reuse the established operation, accepted intent, local binding, native
    # observation and error mappings. The former operation-specific proofs
    # refer to another generated function body and are outside this closure.
    legacy, anchor, _ = legacy.partition('theorem projectionTransport ')
    if not anchor:
        raise ValueError('historical projection theorem is missing')
    legacy = replace_once(legacy, f'import {BASE}.Provider', f'import {BASE}.Types')
    legacy = replace_once(legacy, f'import {PARENT}.FunsExternal', f'import {PARENT}.Types')
    legacy = replace_once(legacy, 'namespace BaseSnapshotLink', f'namespace {LINK}')
    variables = f'variable [p : {PARENT}.Representations]'
    types = ' '.join(SQL_FIELDS.values())
    legacy = replace_once(legacy, variables, f'{variables}\nvariable ({types} : Type)')
    carried = ''.join(f'  {field} := {value}\n' for field, value in SQL_FIELDS.items())
    carried += '  sandproof_protocol_SandboxId := p.sandproof_protocol_SandboxId\n'
    common = '  core_io_error_Error := p.core_io_error_Error'
    legacy = replace_once(legacy, common, carried + common)
    # Pass the extra SQL carriers through each use of the shared type instance.
    old_args = 'SandboxRecord LifecycleReadyObservation'
    for name in (
        'baseRepresentations', 'BIntent', 'BOperation', 'BNative', 'BLocalBinding',
        'BReady', 'BReadiness', 'BError', 'toBaseOperation', 'fromBaseOperation',
        'toBaseIntent', 'fromBaseIntent', 'fromBaseLocal', 'fromBaseNative',
    ):
        legacy = legacy.replace(f'{name} {old_args}', f'{name} {types} {old_args}')
    # The current generated ReadyCandidate carries the native SQLite counter.
    continuation = '    continuation := candidate.continuation\n'
    legacy = replace_once(legacy, continuation, continuation + '    native_revision := candidate.native_revision\n')
    return legacy + theorems


def materialize(base, parent, packages, lean_mcp, output):
    inventory = json.loads((TEMPLATE / 'inputs.json').read_bytes())

    def original(path):
        name = path.relative_to(ROOT).as_posix()
        return checked(ROOT, name, inventory['repository'][name])

    aeneas = subprocess.check_output(['git', '-C', packages / 'aeneas', 'rev-parse', 'HEAD'], text=True).strip()
    if aeneas != PIN or not (lean_mcp / 'bin/python').is_file():
        raise ValueError('fixed Aeneas checkout and existing Lean service environment required')

    files = {}
    for module, raw in ((BASE, base), (PARENT, parent)):
        view = inventory['generated_types'][module]
        if raw.is_symlink() or not raw.is_file():
            raise ValueError('original regular Types.lean file required')
        files[f'{module}/Types.lean'] = apply_view(raw.read_bytes(), view)
    files[f'{BASE}/TypesExternal.lean'] = base_types(
        original(BASE_PROOF / BASE / 'TypesExternal.lean.in').decode()).encode()
    files[f'{PARENT}/TypesExternal.lean'] = original(PARENT_PROOF / PARENT / 'TypesExternal.lean.in')
    foundation = 'SandproofFoundations/TypesExternal.lean'
    files[foundation] = original(ROOT / 'verification/aeneas/foundations' / foundation)
    audit = 'SandboxModel/ProofAuditCommand.lean'
    files[audit] = original(ROOT / audit)
    files[f'{LINK}.lean'] = transport(
        original(BASE_PROOF / 'BaseSnapshotLink.lean.in').decode(),
        (TEMPLATE / f'{LINK}.lean.in').read_text()).encode()
    order = (
        'SandproofFoundations.TypesExternal', f'{BASE}.TypesExternal', f'{PARENT}.TypesExternal',
        f'{BASE}.Types', f'{PARENT}.Types', 'SandboxModel.ProofAuditCommand', LINK,
    )
    modules = {name: dict(sha256=sha(data)) for name, data in files.items()}
    for filename in TOOLS:
        files[filename] = original(ROOT / filename)
    lake = (
        'name = "unload-revision-types"\n\n[[require]]\nname = "aeneas"\n'
        'git = { url = "https://github.com/AeneasVerif/aeneas", subDir = "backends/lean" }\n'
        f'rev = "{PIN}"\n'
    )
    for module in (BASE, PARENT, LINK, 'SandboxModel', 'SandproofFoundations'):
        lake += f'\n[[lean_lib]]\nname = "{module}"\n'
    files['lakefile.toml'] = lake.encode()
    output.mkdir(mode=0o700)
    for name, data in files.items():
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    (output / 'scripts/lean-mcp-server.sh').chmod(0o700)
    (output / '.lake').mkdir()
    (output / '.lake/packages').symlink_to(packages, target_is_directory=True)
    (output / '.tools').mkdir()
    (output / '.tools/lean-mcp').symlink_to(lean_mcp, target_is_directory=True)
    plan = dict(development_only=True, public_acceptance=False, build_order=order, modules=modules,
                roots=[LINK], source_commit=inventory['source_commit'], source_tree=inventory['source_tree'])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ('base', 'parent', 'lake-packages', 'lean-mcp', 'output'):
        parser.add_argument('--' + option, type=Path, required=True)
    args = parser.parse_args()
    materialize(args.base, args.parent, args.lake_packages.resolve(strict=True),
                args.lean_mcp.resolve(strict=True), args.output.resolve())
