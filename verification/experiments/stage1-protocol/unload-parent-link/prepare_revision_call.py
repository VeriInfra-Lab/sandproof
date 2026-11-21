"""Materialize the actual revision reader, parent and precise prefix proofs."""
import argparse
import json
from pathlib import Path

from prepare import HERE, ROOT, STAGE, apply_view, checked, sha
from prepare_revision_transport import BASE, PARENT, BASE_PROOF, PARENT_PROOF, materialize, replace_once


PACKAGE = HERE / 'revision-call'
PROOFS = ('RevisionReadErrors', 'RevisionReadSuccess', 'RevisionParentPrefix', 'RevisionParentCall')


def before_and_after(text, first, last):
    start = text.index(first)
    return text[start:text.index(last, start)]


def parent_prefix(aliases, errors):
    # The original parent body differs from the historical view only in source
    # locations and restored dictionary parameters. Recheck the original proof
    # against the actual body, without importing unrelated historical goals.
    header = """import SandproofUnloadFirstObservation.Funs
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std
namespace RevisionParentPrefix
open SandproofUnloadFirstObservation
variable [Representations] [calls : ParentCalls]

abbrev Intent := sandproof_protocol.lifecycle.authority.AcceptedLifecycleIntent
"""
    prefix = before_and_after(errors, '/-- Changing the final', 'structure FirstBase')
    proof = before_and_after(errors, 'theorem base_error_skips_question',
                             '#audit_controller_proof SandboxModel.Stage1Protocol.UnloadParentLink.Errors.base_error_skips_question')
    missing = before_and_after(errors, 'theorem missing_native_skips_question',
                               '#audit_controller_proof SandboxModel.Stage1Protocol.UnloadParentLink.Errors.missing_native_skips_question')
    variables = """variable {E : Type} {host : host_contract.HostEvidence E} {serde : serde_core.de.DeserializeOwned E}
variable {tx : rusqlite.transaction.Transaction} {ns : Str} {intent : Intent} {now : I64} {input : SourceSnapshot}

"""
    return (
        header + before_and_after(aliases, 'abbrev SourceSnapshot :=', 'abbrev Process :=') + '\n'
        + prefix + variables + proof + missing
        + 'end RevisionParentPrefix\n\n#audit_controller_proof RevisionParentPrefix.base_error_skips_question\n'
        + '#audit_controller_proof RevisionParentPrefix.missing_native_skips_question\n'
    )


def base_effects(legacy, sql, literal):
    fields, definitions = sql.split('-- Definitions in the scope of that dictionary.\n')
    binders = '\nvariable [Representations] [ops : Operations]'
    legacy = replace_once(legacy, binders, fields.split('\n', 1)[1] + binders)
    namespace = f'namespace {BASE}\n\n'
    legacy = replace_once(legacy, namespace, namespace + before_and_after(literal, 'def toStr ', 'def NormalWorldBorrow'))
    return replace_once(legacy, f'end {BASE}', definitions + f'\nend {BASE}')


def prepare_call(base, parent, packages, lean_mcp, output, *, identity=None):
    inventory = json.loads((PACKAGE / 'inputs.json').read_bytes())

    def original(path):
        name = path.relative_to(ROOT).as_posix()
        return checked(ROOT, name, inventory['repository'][name]).decode()

    files = {}
    for namespace, directory in ((BASE, base), (PARENT, parent)):
        raw = directory / 'Funs.lean'
        if raw.is_symlink() or not raw.is_file():
            raise ValueError('original regular Funs.lean required')
        body = apply_view(raw.read_bytes(), inventory['generated_funs'][namespace])
        if namespace == BASE:
            # Rename the admitted dictionary projection and its handwritten
            # counterpart. The checked original generated body is kept intact.
            prefix = 'ArrayT1.Insts.RusqliteParamsParams.'
            body = replace_once(body.decode(), prefix + '__bind_in', prefix + 'bindIn').encode()
        files[f'{namespace}/Funs.lean'] = body
    files[f'{BASE}/FunsExternal.lean'] = base_effects(
        original(BASE_PROOF / BASE / 'FunsExternal.lean.in'),
        (PACKAGE / 'RevisionSqlOperations.lean.in').read_text(),
        original(STAGE / 'create-standard-adapter/FunsExternal.lean.in'),
    ).encode()
    files[f'{PARENT}/FunsExternal.lean'] = original(PARENT_PROOF / PARENT / 'FunsExternal.lean.in').encode()
    files['RevisionParentPrefix.lean'] = parent_prefix(
        original(PARENT_PROOF / 'ParentSuccess.lean.in'), original(HERE / 'ParentErrors.lean.in'),
    ).encode()
    for name in ('RevisionReadErrors', 'RevisionReadSuccess', 'RevisionParentCall'):
        files[f'{name}.lean'] = (PACKAGE / f'{name}.lean.in').read_bytes()

    materialize(base / 'Types.lean', parent / 'Types.lean', packages, lean_mcp, output)
    for name, payload in files.items():
        (output / name).write_bytes(payload)
    lakefile = output / 'lakefile.toml'
    with lakefile.open('a') as stream:
        for name in PROOFS:
            stream.write(f'\n[[lean_lib]]\nname = "{name}"\n')
    plan_file = output / 'plan.json'
    plan = json.loads(plan_file.read_bytes())
    plan['build_order'] += [f'{BASE}.FunsExternal', f'{BASE}.Funs',
                            f'{PARENT}.FunsExternal', f'{PARENT}.Funs', *PROOFS]
    plan['modules'].update({name.removesuffix('.lean').replace('/', '.'): dict(sha256=sha(data))
                            for name, data in files.items()})
    plan['roots'] = ['RevisionParentCall']
    if identity is not None:
        plan.update(source_commit=identity['head'], source_tree=identity['tree'])
    plan_file.write_text(json.dumps(plan, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ('base', 'parent', 'lake-packages', 'lean-mcp', 'output'):
        parser.add_argument('--' + option, type=Path, required=True)
    args = parser.parse_args()
    prepare_call(args.base, args.parent, args.lake_packages.resolve(strict=True),
                 args.lean_mcp.resolve(strict=True), args.output.resolve())
