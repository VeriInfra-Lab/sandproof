"""The three missing Host24 load preparation commands, with original inputs.

Run against a separate exact f3627b7b checkout and fresh artifact directories.
This prepares files; it neither reruns nor relabels historical proof results.
"""
import argparse
import json
from pathlib import Path
import re
import shutil

from prepare import HERE, ROOT, module, sha

DATA = json.loads((ROOT/HERE/'load-reproduction.json').read_text())
MODEL = 'SandboxModel/Lifecycle/Protocol/InboxLoadModel.lean'
SOURCE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def port(project, preparation, output):
    value = json.loads(preparation.read_text())
    if value['source_commit'] != DATA['source_commit']:
        raise ValueError('Host24 load source commit differs')
    for name, change in DATA['proof_changes'].items():
        target = project/name
        if sha(target) != change['original_sha256']:
            raise ValueError('not the original Host23 proof: '+name)
        text = target.read_bytes()
        if len(re.findall(rb'\b23\b', text)) != change['numeric_replacements']:
            raise ValueError('unexpected version literal count: '+name)
        changed = re.sub(rb'\b23\b', b'24', text)
        template = ROOT/HERE.parent/'inbox-load'/(Path(name).name+'.in')
        if changed != template.read_bytes():
            raise ValueError('version-only port differs from checked template: '+name)
        target.write_bytes(changed)
        if sha(target) != change['host24_sha256']:
            raise ValueError('ported proof differs: '+name)
        value['files'][name] = value['installed'][name] = sha(target)
    value.update(proof_changes=DATA['proof_changes'], reason=DATA['reason'], templates_changed=True)
    write(output, value)


def mutate(repository, correct, project, output):
    base = module('load_reproduction_snapshot', ROOT/HERE.parent/'schema-leaves/formal.py')
    extraction = json.loads((correct/'result.json').read_text())
    if (extraction.get('source_commit') != DATA['source_commit']
            or extraction.get('extraction_complete') is not True
            or base.snapshot(repository) != extraction['inputs']):
        raise ValueError('requires exact original f3627b7b source/extraction inputs')
    project.mkdir(mode=0o700, parents=True)
    for name, digest in extraction['inputs'].items():
        target = project/name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(repository/name, target)
        if sha(target) != digest:
            raise ValueError('source changed during preparation: '+name)
    for name, digest in extraction['thin_root_changes'].items():
        if sha(correct/'source'/name) != digest:
            raise ValueError('original transparent projection changed: '+name)
        shutil.copy2(correct/'source'/name, project/name)
    (project/'.tools').symlink_to((correct/'source/.tools').resolve(), target_is_directory=True)
    (project/'.lake').mkdir()
    (project/'.lake/packages').symlink_to((correct/'source/.lake/packages').resolve(), target_is_directory=True)
    old = '    if !matches!(version, crate::LIFECYCLE_STORE_FORMAT_VERSION) {'
    new = '    if !matches!(version, 23) {'
    before = sha(project/SOURCE)
    text = (project/SOURCE).read_text()
    if text.count(old) != 1:
        raise ValueError('present-inbox format guard is not unique')
    (project/SOURCE).write_text(text.replace(old, new, 1))
    if sha(correct/'source'/MODEL) != DATA['same_full_load_model_sha256']:
        raise ValueError('not the same Host24 FullLoadContract')
    changes = dict(extraction['thin_root_changes'])
    changes[SOURCE] = sha(project/SOURCE)
    write(output, dict(kind='actual-host24-stale-present-format-mutation-preparation',
        source_commit=DATA['source_commit'], variant='stale-present-format',
        inputs=extraction['inputs'], thin_root_changes=extraction['thin_root_changes'],
        correct_extraction_result_sha256=sha(correct/'result.json'),
        mutation=dict(path=SOURCE, old=old, new=new, unique_source_match=True,
            original_source_sha256=before, wrong_source_sha256=sha(project/SOURCE)),
        source_changes=changes, same_full_load_model_sha256=DATA['same_full_load_model_sha256'],
        global_format_value_unchanged=24, private_refactoring=False, compile_complete=False,
        extraction_complete=False, formal_counterexample_complete=False,
        native_assertion_complete=False, ning_acceptance=False, shared_acceptance=False, full_v3=False))


def install_counter(project, output):
    driver = module('load_reproduction_original_tools', ROOT/HERE.parent/'inbox-record-validation/formal.py')
    source = ROOT/HERE.parent/'inbox-load/InboxLoadWrongPresentFormat.lean.in'
    if sha(source) != DATA['counter_template_sha256'] or sha(project/MODEL) != DATA['same_full_load_model_sha256']:
        raise ValueError('counterexample or shared contract differs')
    name = 'SandboxModel/Lifecycle/Protocol/InboxLoadWrongPresentFormat.lean'
    target = project/name
    if target.exists():
        raise ValueError('counterexample already installed')
    target.write_bytes(source.read_bytes())
    driver.resolver.ROOT = project
    files = sorted(driver.resolver.closure(project, [name], active_view='lakefile.toml'))
    audits = [a for n in files for a in re.findall(r'^#audit_controller_proof (\S+)$', (project/n).read_text(), re.M)]
    if len(files) != 108 or len(audits) != 424 or len(set(audits)) != 424:
        raise ValueError('counterexample physical closure differs')
    write(output, dict(files=files, expected_audits=audits, new_file=name,
        new_file_sha256=sha(target), source_commit=DATA['source_commit'],
        same_full_load_model_sha256=DATA['same_full_load_model_sha256']))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='stage', required=True)
    command = commands.add_parser('port')
    command.add_argument('--preparation', type=Path, required=True)
    command = commands.add_parser('mutate')
    command.add_argument('--repository', type=Path, required=True)
    command.add_argument('--correct', type=Path, required=True)
    commands.add_parser('install-counter')
    for command in commands.choices.values():
        command.add_argument('--project', type=Path, required=True)
        command.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.stage == 'port':
        port(args.project.absolute(), args.preparation.absolute(), args.output.absolute())
    elif args.stage == 'mutate':
        mutate(args.repository.absolute(), args.correct.absolute(), args.project.absolute(), args.output.absolute())
    else:
        install_counter(args.project.absolute(), args.output.absolute())
