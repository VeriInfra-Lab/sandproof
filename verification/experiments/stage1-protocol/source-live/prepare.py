"""Materialize separate complete Lean views; preparation is not acceptance."""
import argparse
import json
from pathlib import Path
import re
import shutil

import support

HERE = Path(__file__).resolve().parent
PROOF = Path('SandboxModel/Lifecycle/Protocol/SourceLive')
GEN = Path('verification/aeneas/stage1-protocol/source-live')
NEGATIVE = {'skip-version': 'NegativeVersion', 'allow-expired': 'NegativeDeadline',
            'ignore-startup': 'NegativeStartup'}
CORRECT = ('Spec', 'Guard', 'Started', 'Core', 'Values', 'Examples', 'Witnesses', 'Counterexamples')
COMMON = ('Spec', 'Values', 'Examples', 'Counterexamples')


def inventory(root):
    support.source_guard(root)
    data = json.loads((HERE / 'inventory-proof.json').read_text())
    support.require(data['source_base'] == support.BASE and data['adopted'] is False,
                    'source-live candidate scope differs')
    support.require(tuple(data['generated']) == ('correct', *NEGATIVE), 'missing or extra actual view')
    for section in ('providers', 'templates', 'drivers'):
        for name, wanted in data[section].items():
            path = (root if section == 'providers' else HERE) / name
            support.require(path.is_file() and not path.is_symlink() and support.sha(path) == wanted,
                            'source-live proof input changed: ' + name)
    for view, files in data['generated'].items():
        support.require(set(files) == {'Types.lean', 'Funs.lean'}, 'generated view is incomplete')
        for name, wanted in files.items():
            path = root / GEN / view / 'SandproofSourceLive' / name
            support.require(path.is_file() and not path.is_symlink() and support.sha(path) == wanted,
                            'actual generated view changed: ' + str(path))
    return data


def prepare(root, output):
    data = inventory(root)
    output.mkdir(parents=True, exist_ok=False)
    runtime = support.module('source_live_prepare_runtime', root /
                             'verification/experiments/stage1-protocol/formal_runtime.py')
    result = {}
    for view in data['generated']:
        project = output / view
        project.mkdir()

        def copy(source, target):
            target = project / target
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
            support.require(support.sha(source) == support.sha(target), 'proof copy changed')

        for name in data['providers']:
            if name.endswith('.lean') or name in ('lean-toolchain', 'lake-manifest.json',
                    'tools/lean_session.py', 'tools/proof_services.py', 'scripts/lean-mcp-server.sh'):
                copy(root / name, name)
        generated = GEN / view / 'SandproofSourceLive'
        for name in data['generated'][view]:
            copy(root / generated / name, generated / name)
        copy(HERE / 'FunsExternal.lean.in', generated / 'FunsExternal.lean')
        names = CORRECT if view == 'correct' else (*COMMON, NEGATIVE[view])
        for name in names:
            copy(HERE / (name + '.lean.in'), PROOF / (name + '.lean'))
        lake = (HERE / 'lakefile.toml.in').read_text().replace('@VIEW@', view)
        (project / 'lakefile.toml').write_text(lake)
        (project / '.tools').symlink_to(runtime.tool_root(root), target_is_directory=True)
        (project / '.lake').mkdir()
        (project / '.lake/packages').symlink_to(runtime.package_root(root), target_is_directory=True)
        actual = sorted(str(p.relative_to(project)) for tree in ('SandboxModel', 'verification/aeneas')
                        for p in (project / tree).rglob('*.lean'))
        audits = [name for path in actual for name in re.findall(
            r'^#audit_controller_proof (\S+)$', (project / path).read_text(), re.M)]
        support.require(len(audits) == len(set(audits)) and audits == data['audits'][view],
                        'missing, duplicate or changed source-live proof audit')
        targets = ('Witnesses', 'Counterexamples') if view == 'correct' else (NEGATIVE[view],)
        result[view] = dict(project=str(project), files=actual, audits=audits,
            targets=['SandboxModel.Lifecycle.Protocol.SourceLive.' + name for name in targets],
            inputs={p: support.sha(project / p) for p in actual})
    (output / 'prepared.json').write_text(json.dumps(dict(views=result,
        prepared_complete=True, formal_acceptance=False, ning_acceptance=False), indent=2) + '\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    prepared = prepare(support.ROOT, args.output.resolve())
    print(json.dumps(dict(prepared_views=list(prepared), formal_acceptance=False)))
