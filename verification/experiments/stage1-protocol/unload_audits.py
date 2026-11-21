"""The four fixed unload audit packages and their original inputs."""
import hashlib
import importlib.util
import json
from pathlib import Path
import re

HERE = Path('verification/experiments/stage1-protocol')
LABEL = 'stage1-unload-audits'
KIND = 'stage1-unload-audits-component'
DRIVER = HERE / 'unload_audit_driver.py'
PARENT_LABEL = 'stage1-unload-parent-transaction'
PARENT_DRIVER = HERE / 'unload_parent_transaction.py'
PARENT_CASES = ('begin', 'prefix', 'prefix-reordered', 'finish', 'finish-reordered', 'revision', 'parent-link')
PARENT_AUDIT_INPUTS = {
    'begin': {'started-transaction/UnloadStartedTransaction', 'world-begin/UnloadWorldBegin'},
    'prefix': {'frontier/UnloadFrontierInputs', 'frontier/UnloadFrontier'},
    'prefix-reordered': {'frontier/UnloadFrontierInputs', 'frontier/UnloadFrontierReordered'},
    'finish': {'finish/UnloadFinishInputs', 'finish/UnloadFinish'},
    'finish-reordered': {'finish/UnloadFinishInputs', 'finish/UnloadFinishReordered'},
    'revision': {'revision-transport/RevisionReadyTransport', 'revision-call/RevisionReadErrors',
                 'revision-call/RevisionReadSuccess', 'revision-call/RevisionParentCall',
                 'ParentErrors', 'ParentRejects'},
}
AUDIT = re.compile(r'^#audit_controller_proof (\S+)$', re.M)
PACKAGES = {
    'unload-startup-source': ('startup', 16, 34),
    'unload-base-provenance': ('base-provenance', 23, 52),
    'unload-continued-observation': ('continued-observation', 2, 6),
    'unload-stored-options': ('stored-options', 3, 11),
}


def check_parent_audits(root, result, folder):
    if result.get('kind') != PARENT_LABEL or tuple(result.get('cases', {})) != PARENT_CASES:
        raise ValueError('missing actual parent transaction formal cases')
    package = root / HERE / 'unload-parent-link'
    common = PARENT_AUDIT_INPUTS['begin']
    for case, actual in result['cases'].items():
        if case == 'parent-link':
            expected = {path.name.removesuffix('.lean.in') for path in package.glob('*.lean.in')}
        else:
            expected = PARENT_AUDIT_INPUTS[case]
        if case not in ('revision', 'parent-link'):
            expected = common | expected
        names = set()
        for path in sorted(expected):
            template = package / (path + '.lean.in')
            page = Path(path).name + '.lean'
            files = [(name, sha) for name, sha in actual.get('files', {}).items()
                     if Path(name).name == page]
            if len(files) != 1:
                raise ValueError('missing or changed actual parent audit page: ' + page)
            name, sha = files[0]
            if path == 'revision-transport/RevisionReadyTransport':
                # This page appends the template to the materialized carrier definitions.
                if folder is not None:
                    materialized = folder / case / 'project' / name
                    if (digest(materialized) != sha
                            or not materialized.read_bytes().endswith(template.read_bytes())):
                        raise ValueError('missing actual revision transport declarations')
            elif sha != digest(template):
                raise ValueError('missing or changed actual parent audit page: ' + page)
            names.update(AUDIT.findall(template.read_text()))
        if (actual.get('complete') is not True
                or not re.fullmatch(r'[0-9a-f]{64}', actual.get('service_log_sha256', ''))
                or not names <= set(actual.get('audits', ())) or not actual.get('files')):
            raise ValueError('incomplete parent transaction formal audits: ' + case)


def check_parent_captures(folder, result, evidence):
    captures = [('begin', 'result.json', 'head', 'tree', 'complete')]
    captures += [('frontier-' + name, 'probe-result.json', 'source_commit', 'source_tree', 'generated')
                 for name in ('prefix', 'parent', 'reordered')]
    captures += [('finish-' + name, 'result.json', 'source_commit', 'source_tree', 'complete')
                 for name in ('tail', 'parent', 'reordered')]
    captures += [('revision-' + name, 'result.json', 'head', 'tree', 'complete')
                 for name in ('base-revision', 'parent')]
    for name, receipt, commit, tree, complete in captures:
        capture = folder / 'artifacts' / 'captures' / name
        report_path = capture / receipt
        report = json.loads(report_path.read_bytes())
        if (evidence.get(str(report_path)) != digest(report_path)
                or report.get(complete) is not True
                or report.get(commit) != result['commit'] or report.get(tree) != result['tree']):
            raise ValueError('missing current parent extraction receipt: ' + name)
        paths = {'actual.llbc': report['ir_sha256'], 'source.tar': result['source_sha256']}
        paths.update({row['label'] + '.log': row['log_sha256'] for row in report['commands']})
        generated = report['generated'] if commit == 'head' else report['generated_sha256']
        paths.update({'generated/' + path: sha for path, sha in generated.items()})
        if any(evidence.get(str(capture / path)) != sha for path, sha in paths.items()):
            raise ValueError('missing or changed current parent extraction inputs: ' + name)


def parent_audit_pages(root, templates, actuals, composed_pages):
    """Count every declared audit on its own physically checked page."""
    package = HERE / 'unload-parent-link'
    observations = {}
    for actual in actuals:
        for page, sha in actual.get('files', {}).items():
            key = (Path(page).name, sha)
            observations.setdefault(key, set()).update(actual['audits'])
    pages = {name: audits for name, audits in templates.items() if Path(name).is_relative_to(package)}
    missing = []
    for name, audits in pages.items():
        page = Path(name).with_suffix('').name
        sha = composed_pages.get(page) or digest(root / name)
        if not set(audits) <= observations.get((page, sha), set()):
            missing.append(name)
    return dict(pages=len(pages), declarations=sum(map(len, pages.values())), missing=missing)


def digest(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('missing or linked original unload input: ' + str(path))
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def frozen_inputs(root):
    """Keep the existing reviewed Rust, extraction and proof files pinned."""
    inputs = {}
    for suffix, _, _ in PACKAGES.values():
        manifest = Path('verification') / f'v3-unload-{suffix}-inputs.sha256'
        inputs[str(manifest)] = digest(root / manifest)
        for line in (root / manifest).read_text().splitlines():
            expected, name = line.split(maxsplit=1)
            relative = Path(name)
            if (relative.is_absolute() or '..' in relative.parts
                    or not (root / relative).resolve().is_relative_to(root)
                    or digest(root / relative) != expected):
                raise ValueError('original unload frozen input differs: ' + name)
            inputs[name] = expected
    return inputs


def package_audits(templates, package):
    _, count, declarations = PACKAGES[package]
    directory = HERE / package
    files = {name: entries for name, entries in templates.items()
             if Path(name).is_relative_to(directory)}
    names = [name for entries in files.values() for name in entries]
    if len(files) != count or len(names) != declarations or len(set(names)) != declarations:
        raise ValueError('missing, duplicate or extra fixed unload audit: ' + package)
    return files


def check_current_captures(root, folder, result, evidence):
    spec = importlib.util.spec_from_file_location('current_unload_captures',
                                                 root/HERE/'unload_current_capture.py')
    capture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(capture)
    artifacts = folder/'artifacts'
    leaves = result['current_captures']
    if set(leaves) != set(capture.NAMES):
        raise ValueError('missing current Unload leaf captures')
    for case, files in leaves.items():
        path = artifacts/'current-captures'/case
        if files != capture.admit(root, path, case, digest) or any(
                evidence.get(str(path/name)) != sha for name, sha in files.items()):
            raise ValueError('missing current Unload leaf raw evidence: ' + case)
    pair = artifacts/'revision-pair.json'
    if evidence.get(str(pair)) != digest(pair):
        raise ValueError('missing current Unload revision comparison')
    for case in ('base-revision', 'parent'):
        path = artifacts/('revision-' + case)
        receipt = path/'result.json'
        row = json.loads(receipt.read_bytes())
        if (evidence.get(str(receipt)) != digest(receipt) or not row['complete']
                or row['head'] != result['commit'] or row['tree'] != result['tree']):
            raise ValueError('current Unload revision came from another source: ' + case)
        fields = {'actual.llbc': row['ir_sha256'], 'source.tar': result['source_sha256']}
        fields.update({command['label'] + '.log': command['log_sha256'] for command in row['commands']})
        fields.update({'generated/' + name: sha for name, sha in row['generated'].items()})
        if any(evidence.get(str(path/name)) != sha for name, sha in fields.items()):
            raise ValueError('incomplete current Unload revision raw evidence: ' + case)
