"""Instantiate existing generic proofs and exact per-symbol value bindings.

Only import/module/provider/callee names change. No old Manifest generated
type module is imported and no specification or proof step is replaced.
"""
from hashlib import sha256
from pathlib import Path
import re

GENERIC = ('LoopWitness', 'ManifestPathModel', 'ManifestPath',
           'ManifestProfileModel', 'ManifestProfile')
SOURCE_PREFIX = Path('SandboxModel/Lifecycle/Protocol')
STD = (
    ('verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean', (
        'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.vec.Vec.is_empty',
        'core.str.Str.is_empty', 'core.str.Str.len',
        'alloc.string.String.Insts.CoreOpsDerefDerefStr.deref',
        'Str.Insts.CoreCmpPartialEqStr.eq',
        'alloc.string.String.Insts.CoreCmpPartialEqShared0Str.eq')),
    ('verification/experiments/stage1-protocol/inbox-validation/SharedValueBindings.lean.in', (
        'core.str.Str.bytes', 'core.str.iter.Bytes.Insts.CoreIterTraitsIteratorIteratorU8.all')),
    ('verification/experiments/stage1-protocol/inbox-config-validation/FunsExternal.lean.in', (
        'alloc.string.String.len', 'alloc.string.String.is_empty')),
)
STD_NAMESPACE = 'SandboxModel.Stage1Protocol.SourceFullWorkGuardStd'
ACTUAL_NAMESPACE = 'SandboxModel.Stage1Protocol.SourceFullWorkGuardActual'
PROVIDER_NAMES = { 'SandproofFoundationsRust.' + name: STD_NAMESPACE + '.' + name
                   for _, names in STD for name in names }


def digest(data):
    return sha256(data).hexdigest()


def providers(text):
    for original, current in PROVIDER_NAMES.items():
        text = text.replace(original, current)
    return text


def standards(repo):
    bodies, provenance = [], []
    for relative, names in STD:
        raw = (repo / relative).read_bytes()
        for name in names:
            starts = list(re.finditer(rb'^def ' + re.escape(name.encode()) + rb'(?=[\s({:])', raw, re.M))
            if len(starts) != 1:
                raise ValueError('one exact existing standard declaration is required: ' + name)
            begin = starts[0].start()
            end = raw.find(b'\n\n', begin)
            if end < begin:
                raise ValueError('the complete existing standard declaration is required')
            original = raw[begin:end]
            if re.search(rb'\n(?:def|abbrev|end)\b', original):
                raise ValueError('a selected standard declaration cannot include another declaration')
            instantiated = providers(original.decode()).encode()
            bodies.append(instantiated)
            provenance.append(dict(source=relative, source_sha256=digest(raw), symbol=name,
                original_declaration=digest(original), instantiated_declaration=digest(instantiated),
                allowed_changes='exact listed provider qualified names only'))
    if len(bodies) != 11:
        raise ValueError('all11 existing standard declarations are mandatory')
    return b'\n\n'.join(bodies), provenance


def generic(repo, actual_symbols):
    module_names = { 'SandboxModel.Lifecycle.Protocol.' + name: 'SourceFullWorkGuard' + name
                     for name in GENERIC }
    namespaces = { 'SandboxModel.Stage1Protocol.' + name:
                   'SandboxModel.Stage1Protocol.SourceFullWorkGuard' + name
                   for name in ('ManifestLoop', 'ManifestPath', 'ManifestProfile') }
    pattern = re.compile(r'(?<![\w.])(' + '|'.join(map(re.escape,
        sorted(actual_symbols, key=lambda name: (-len(name), name)))) + r')(?![\w.])')
    files, provenance = {}, {}
    for name in GENERIC:
        relative = SOURCE_PREFIX / (name + '.lean')
        raw = (repo / relative).read_bytes()
        text = raw.decode()
        for original, current in module_names.items():
            text = text.replace('import ' + original + '\n', 'import ' + current + '\n')
        text = text.replace('import SandproofStage1Manifest.Funs\n', 'import SourceFullWorkGuardView\n')
        text = text.replace('import SandproofStage1Manifest.Types\n', 'import SourceFullWorkGuardTypes\n')
        for original, current in namespaces.items():
            text = text.replace(original, current)
        opened = ('SandproofSourceRetentionProviders\n'
                  'open SandboxModel.Stage1Protocol SourceRetireTypes SourceFullWorkGuardTypes')
        if not name.endswith('Model'):
            opened += ' SourceFullWorkGuardActual'
        text = text.replace('SandproofStage1ManifestRust', opened)
        text = providers(text)
        text = pattern.sub(lambda match: ACTUAL_NAMESPACE + '.' + match.group(1), text)
        target = 'SourceFullWorkGuard' + name + '.lean'
        files[target] = text.encode()
        provenance[str(relative)] = dict(source_sha256=digest(raw), materialized=target,
            sha256=digest(files[target]),
            allowed_changes='exact imports, module namespaces, opens, listed provider and actual callee qualified names only',
            proof_logic_unchanged=True, specification_logic_unchanged=True)
    return files, provenance


def input_paths():
    return [*(str(SOURCE_PREFIX / (name + '.lean')) for name in GENERIC),
            *(relative for relative, _ in STD)]
