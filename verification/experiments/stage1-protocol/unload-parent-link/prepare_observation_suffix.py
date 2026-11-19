"""Check and bind the exact retained observation-save continuation."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import re
import shutil

import observation_slot_capture as slot
from prepare_observation_finish import MODULE as FINISH_MODULE, captured as finish_capture
from prepare_observation_prefix import generated_declarations, parent_relation, verified
from prepare_observation_slot import SUPPORT, captured as slot_capture, definitions as slot_definitions
from prepare_retention_providers import declarations
from prepare_world_begin import closure


HERE = Path(__file__).with_name('observation-suffix')
PREFIX = 'controller.lifecycle_inbox.source_work.'
PARENT = PREFIX + 'ControllerJournal.save_source_observation'
FINISH = PREFIX + 'finish_observation_save'
FINISH_SOURCE = PREFIX + 'finish_source_work'
SCAN_DICTIONARY = (PREFIX + 'JournalObservationWork.Insts.'
                   'Sandproof_controller_serviceControllerLifecycle_inboxSource_work'
                   'ObservationWorkSourceWorkRecordSourceWork')
SAVE_DICTIONARY = (PREFIX + 'SaveSourceInputsToJournal.Insts.'
                   'Sandproof_controller_serviceControllerLifecycle_inboxSource_workSaveSourceInputsSources')
TYPES = (
    ('CrashPoint', None),
    ('CrashInjector', 'CrashInjector c'),
    ('controller.lifecycle_inbox.model.LifecycleInboxConfig',
     'controller.lifecycle_inbox.model.LifecycleInboxConfig c'),
    ('controller.lifecycle_inbox.model.Inbox', None),
    (PREFIX + 'SourceWorkRecord', PREFIX + 'SourceWorkRecord c'),
    (PREFIX + 'codec.Sources', None),
    (PREFIX + 'SaveSourceInputs', None),
    ('journal.Journal', 'journal.Journal c'),
    ('controller.ControllerJournal', None),
)
MARKER = b'  let cf \xe2\x86\x90 core.result.Result.Insts.CoreOpsTry.branch r\n'
SPLICED = ('ObservationSuffixTypes', 'ObservationSuffixGenerated')
ROOT = 'ObservationSuffix'


def replace_once(document, marker, block):
    if document.count(marker) != 1:
        raise ValueError('exact continuation template insertion is required: ' + marker.decode())
    return document.replace(marker, block)


def blocks(parent):
    functions = generated_declarations(parent['Funs.lean'])
    types = generated_declarations(parent['Types.lean'])
    whole = functions[PARENT]
    if (whole.count(MARKER) != 1 or whole.count(b'  let (r, c)') != 1
            or b'Result (core.result.Result (T \xc3\x97 I \xc3\x97 R) E)'
            not in functions[PREFIX + 'source_observation_prefix']):
        raise ValueError('the retained parent or the generic Begin writeback mismatch changed')
    head, continuation = whole.split(MARKER)
    if b'    controller.lifecycle_inbox.source_work.source_observation_prefix' not in head:
        raise ValueError('the retained suffix must begin immediately after the original prefix invocation')
    frozen_types = []
    for name, notation in TYPES:
        frozen_types.append(types[name])
        if notation:
            frozen_types.append(f'local notation "{name}" => {notation}'.encode())
    adapters = []
    for dictionary, member in ((SCAN_DICTIONARY, 'same_work'), (SAVE_DICTIONARY, 'save')):
        name = f'{dictionary}.{member}'
        adapters.extend((functions[name], f'local notation "{name}" => {name} calls'.encode(),
                         functions[dictionary]))
    replacements = {
        b'-- GENERATED TYPES': b'\n\n'.join(frozen_types),
        b'-- GENERATED ADAPTERS': b'\n\n'.join(adapters),
        b'-- GENERATED FINISH SOURCE': functions[FINISH_SOURCE],
        b'-- GENERATED FINISH OBSERVATION': functions[FINISH],
        b'-- GENERATED SUFFIX': MARKER + continuation,
    }
    return functions, replacements


def prepare(parent, scan, tail, output, packages, service):
    parent_report, parent_bodies = verified(parent, 'parent')
    relation = parent_relation(parent / 'actual.llbc', parent_report['admission'])
    slot_bodies, slot_report = slot_capture(scan, 'core')
    finish_files, finish_report = finish_capture(tail, 'tail')
    functions, replacements = blocks(parent_bodies)
    for file in ('Types.lean', 'Funs.lean'):
        if slot_definitions(slot_bodies[file], file) != slot_definitions(parent_bodies[file], file):
            raise ValueError('the retained actual scan and decision differ from the audited view')
    finish_body = finish_files[f'{FINISH_MODULE}/Funs.lean'].removesuffix(
        b'\n\nend ' + FINISH_MODULE.encode() + b'\n')
    finish_definitions = declarations(finish_body)
    if any(finish_definitions[name] != functions[name] for name in (FINISH, FINISH_SOURCE)):
        raise ValueError('the retained actual tail differs from #959')
    output.mkdir(mode=0o700)
    for relative, target in (('.lake/packages', packages), ('.tools/lean-mcp', service)):
        path = output / relative
        path.parent.mkdir()
        path.symlink_to(target, target_is_directory=True)
    for name in (*SUPPORT, 'tools/lean_session.py', 'scripts/lean-mcp-server.sh', 'lake-manifest.json', 'lean-toolchain'):
        path = output / name
        path.parent.mkdir(exist_ok=True, parents=True)
        shutil.copy2(slot.ROOT / name, path)
    with (output / 'lakefile.toml').open('x') as file:
        pinned = (slot.ROOT / 'lakefile.toml').read_text()
        dependency, = re.findall(r'\[\[require\]\]\nname = "aeneas"\n[^\n]+\n[^\n]+\n', pinned)
        file.write('name = "source-observation-suffix"\n' + dependency)
        for library in ('SandboxModel', slot.NAMESPACE, 'ObservationSlotInputs', 'ObservationSlot', *SPLICED, ROOT):
            file.write(f'\n[[lean_lib]]\nname = "{library}"\n')
    generated = output / slot.NAMESPACE
    generated.mkdir()
    for name, body in slot_bodies.items():
        (generated / name).write_bytes(body)
    for module in ('ObservationSlotInputs', 'ObservationSlot'):
        shutil.copyfile(slot.HERE / (module + '.lean.in'), output / (module + '.lean'))
    for module in SPLICED:
        document = (HERE / (module + '.lean.in')).read_bytes()
        for marker, block in replacements.items():
            if module.endswith('Types') == (marker == b'-- GENERATED TYPES'):
                document = replace_once(document, marker, block)
        (output / (module + '.lean')).write_bytes(document)
    shutil.copyfile(HERE / (ROOT + '.lean.in'), output / (ROOT + '.lean'))
    files = closure(output, [ROOT + '.lean'], active_view='lakefile.toml')
    expected = {*SUPPORT, *(f'{slot.NAMESPACE}/{name}' for name in slot_bodies),
                *(name + '.lean' for name in ('ObservationSlotInputs', 'ObservationSlot', *SPLICED, ROOT))}
    if files.keys() != expected:
        raise ValueError('the actual continuation source closure differs')
    plan = dict(root=ROOT, source=slot.finish.SOURCE, relation=relation, parent=parent_report['generated_sha256'],
                scan=slot_report, tail=finish_report['generated_sha256'],
                spliced={key.decode(): sha256(body).hexdigest() for key, body in replacements.items()},
                files=files, strict_files=sorted(expected),
                modules={name: {'sha256': digest} for name, digest in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(ROOT, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('parent', 'scan', 'tail', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    prepare(args.parent.resolve(strict=True), args.scan.resolve(strict=True),
            args.tail.resolve(strict=True), args.output.resolve(), args.lake_packages.resolve(strict=True),
            args.lean_mcp.resolve(strict=True))
