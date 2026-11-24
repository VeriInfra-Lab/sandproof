"""Admit the exact generated views and use their raw checked declarations."""

from hashlib import sha256
import json
from pathlib import Path
import re
import tarfile
import tempfile

from encoder_capture import admit
from encoder_inputs import CODEC, HERE, NAMESPACE, ROOT, configuration, retention, stage
from joint_prepare import ALIGNED, NAMESPACE as PARENT_NAMESPACE, declarations


def checked(folder, mode, row):
    receipt = json.loads((folder / 'result.json').read_bytes())
    drivers = {name: retention.sha(HERE / name) for name in ('encoder_capture.py', 'encoder_inputs.py')}
    if (receipt['complete'] is not True or receipt['public_acceptance'] is not False
            or (receipt['mode'], receipt['source'], receipt['configuration'], receipt['driver'])
            != (mode, retention.SOURCE, configuration(row), drivers)
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in receipt['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('complete actual encoder capture is required')
    for name, digest in [('source.tar', retention.SOURCE_ARCHIVE), ('actual.llbc', receipt['ir']), *[
            (entry['label'] + '.log', entry['log_sha256']) for entry in receipt['commands']]]:
        if retention.sha(folder / name) != digest:
            raise ValueError('capture input changed: ' + name)
    with tempfile.TemporaryDirectory(dir=folder.parent, prefix='encoder-verify-') as directory:
        expected = Path(directory)
        with tarfile.open(folder / 'source.tar') as archive:
            archive.extractall(expected, filter='data')
        stage(row, expected, mode)
        actual = folder / 'source'
        names = {p.relative_to(expected) for p in expected.rglob('*') if p.is_file()}
        if names != {p.relative_to(actual) for p in actual.rglob('*') if p.is_file()}:
            raise ValueError('actual encoder source inventory changed')
        for name in names:
            if (actual / name).is_symlink() or retention.sha(actual / name) != retention.sha(expected / name):
                raise ValueError('actual encoder source changed: ' + str(name))
    generated = folder / 'generated'
    names = {'translation.json', *(f'{NAMESPACE}/{name}' for name in
        (*ALIGNED, 'TypesExternal_Template.lean', 'FunsExternal_Template.lean'))}
    found = {str(p.relative_to(generated)): retention.sha(p) for p in generated.rglob('*') if p.is_file()}
    if names != found.keys() or found != receipt['generated']:
        raise ValueError('actual encoder output changed')
    target = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if admit(row, folder / 'actual.llbc', generated, target) != receipt['admission']:
        raise ValueError('actual encoder admission changed')
    return {name: (generated / NAMESPACE / name).read_bytes() for name in ALIGNED}


def correspondence(correct, wrong, parent):
    matches = {}
    for name in ALIGNED:
        good, bad = (declarations(mode[name], NAMESPACE) for mode in (correct, wrong))
        shared = declarations(parent[name], PARENT_NAMESPACE)
        if not shared or any(good.get(symbol) != body for symbol, body in shared.items()):
            raise ValueError('original full parent declarations differ: ' + name)
        if good.keys() != bad.keys():
            raise ValueError('encoder mutation changed the declaration inventory')
        changed = {symbol for symbol in good if good[symbol] != bad[symbol]}
        if name == 'Types.lean':
            if changed:
                raise ValueError('encoder mutation changed generated data types')
        else:
            step = CODEC + 'encode_candidate_records_loop.body'
            original = 'let encoded1 ← alloc.vec.Vec.push encoded (key, bytes1)'.encode()
            counterexample = 'let a := Array.repeat 32#usize 0#u8\n              let encoded1 ← alloc.vec.Vec.push encoded (a, bytes1)'.encode()
            if changed != {step} or good[step].count(original) != 1 or good[step].replace(original, counterexample) != bad[step]:
                raise ValueError('mutation must change only the key in the original emitted row')
        matches[name] = dict(parent=len(shared), encoder=len(good))
    return matches


def raw_declarations(data):
    result = {}
    for block in data.split(b'\n\n/--'):
        match = re.search(rb'^(?:def|impl_def|structure|inductive)\s+([^\s(:]+)', block, re.M)
        if match:
            attributes_at = match.start()
            comment_end = block.rfind(b'-/\n', 0, match.start())
            if comment_end != -1:
                attributes_at = comment_end + 3
            name = match[1].decode()
            if name in result:
                raise ValueError('duplicate generated definition: ' + name)
            result[name] = block[attributes_at:].removesuffix(b'\n\nend ' + NAMESPACE.encode() + b'\n')
    return result


def materialize(output, capture, positive, mode):
    from prepare_world_begin import closure
    output.mkdir(mode=0o700)
    (output / '.lake').mkdir()
    (output / '.tools').mkdir()
    for name in ('.lake/packages', '.tools/lean-mcp'):
        (output / name).symlink_to((positive / name).resolve(strict=True))

    def put(name, data):
        file = output / name
        file.parent.mkdir(parents=True, exist_ok=True)
        with file.open('xb') as handle:
            handle.write(data)

    provider = json.loads((positive / 'plan.json').read_bytes())
    for name, digest in provider['files'].items():
        data = (positive / name).read_bytes()
        if sha256(data).hexdigest() != digest:
            raise ValueError('proved parent file differs: ' + name)
        put(name, data)
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        put(name, (positive / name).read_bytes())
    types, funs = (raw_declarations(capture[name]) for name in ALIGNED)
    codec = CODEC
    instance = next(k for k in funs if k.startswith(codec + 'JournalSourceSave.Insts.') and k.endswith('.encode')).removesuffix('.encode')
    key_instance = next(k for k in funs if k.startswith(codec + 'JournalSavedKeys.Insts.') and k.endswith('.contains')).removesuffix('.contains')

    def joined(names):
        return b'\n\n'.join(funs[name] for name in names)

    data = b'\n\n'.join(types[name] for name in ('core.borrow.Borrow', codec + 'JournalSavedKeys', codec + 'JournalSourceSave'))
    data += b'\n\ndef core.borrow.Borrow.Blanket.borrow {T : Type} (v : T) : Result T := .ok v\n\n'
    data += joined(('core.borrow.Borrow.Blanket', 'capture.retirement.MATERIAL_RETIREMENT_PLAN_MAX_BYTES',
                    'capture.retirement.MATERIAL_RETIREMENT_RECORD_MAX_BYTES', 'capture.retirement.MATERIAL_RELEASE_RECORD_MAX_BYTES',
                    'capture.source.SourceObservationConfig.required_bytes',
                    'capture.source.SourceObservationConfig.Insts.CoreCmpPartialEqSourceObservationConfig',
                    'sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity'))
    wiring = b''
    for part in ('contains', ''):
        name = key_instance + ('.' + part if part else '')
        wiring += funs[name] + b'\n\n' + f'local notation "{name}" => {name} ports\n\n'.encode()
    wiring += funs[codec + 'retains_saved_records'] + b'\n\n'
    wiring += f'local notation "{codec}retains_saved_records" => {codec}retains_saved_records ports\n\n'.encode()
    for part in ('read', 'limit', 'encode', 'retains', 'write', ''):
        name = instance + ('.' + part if part else '')
        wiring += funs[name] + b'\n\n'
        if part != 'write':
            wiring += f'local notation "{name}" => {name} ports\n\n'.encode()
    wiring += funs[codec + 'save']
    ports = (HERE / 'EncoderPorts.lean.in').read_bytes()
    for marker, fragment in (
        (b'DATA_AND_ARITHMETIC', data), (b'ACTUAL_WIRING', wiring),
        (b'ACTUAL_STEP', funs[codec + 'encode_candidate_records_loop.body']),
        (b'ACTUAL_LOOP', funs[codec + 'encode_candidate_records_loop']),
        (b'ACTUAL_ENTRY', funs[codec + 'encode_candidate_records']),
    ):
        assert ports.count(b'-- ' + marker) == 1
        ports = ports.replace(b'-- ' + marker, fragment)
    ports += (f'\nnamespace SandproofSaveRetentionRows\n'
              f'abbrev encoderJournal := @{instance}\n'
              f'abbrev encoderKeyCheck := @{key_instance}\n'
              f'end SandproofSaveRetentionRows\n').encode()
    put('EncoderPorts.lean', ports)
    lake = (positive / 'lakefile.toml').read_text()
    for name in ('EncoderPorts', 'EncoderSpec', 'EncoderWitness', 'EncoderWitnessRun', 'EncoderCheck'):
        lake += f'\n[[lean_lib]]\nname = "{name}"\n'
    for name in ('EncoderSpec', 'EncoderWitness'):
        put(name + '.lean', (HERE / (name + '.lean')).read_bytes())
    procedure = (HERE / 'EncoderWitnessRun.lean.in').read_bytes()
    output_key = b'(keyOf row)' if mode == 'correct' else b'(Array.repeat 32#usize 0#u8)'
    put('EncoderWitnessRun.lean', procedure.replace(b'OUTPUT_KEY', output_key))
    if mode == 'correct':
        for name in ('EncoderProof', 'EncoderFirstError', 'EncoderComposition'):
            put(name + '.lean', (HERE / (name + '.lean')).read_bytes())
            lake += f'\n[[lean_lib]]\nname = "{name}"\n'
    put('EncoderCheck.lean', (HERE / ('EncoderCheck-' + mode + '.lean.in')).read_bytes())
    put('lakefile.toml', lake.encode())
    files = closure(output, ['EncoderCheck.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (output / file).read_text(), re.M)})
    plan = dict(root='EncoderCheck', mode=mode, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': value} for name, value in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files], public_acceptance=False)
    retention.write_json(output / 'plan.json', plan)
    return plan
