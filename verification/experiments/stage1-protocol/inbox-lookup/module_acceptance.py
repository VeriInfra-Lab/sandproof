"""Select and run the local Inbox Lookup obligation without a full Stage1 seal."""

import argparse
from collections import defaultdict
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time
import tomllib

ROOT = Path(__file__).resolve().parents[4]
STAGE1 = Path('verification/experiments/stage1-protocol')
HERE = STAGE1 / 'inbox-lookup'
MODULE = 'inbox-lookup'
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
WORKFLOW_DOC = 'docs/proof-workflow.md'
ISOLATED_INPUTS = {
    str(HERE / 'module_acceptance.py'),
    str(HERE / 'InboxLookupWrong.lean.in'),
    'tools/tests/test_inbox_lookup_module_acceptance.py',
}
VARIANTS = {
    'correct': ('candidate', 'InboxLookup', 'InboxLookup.find_contract'),
    'key-only': ('key-only', 'InboxLookupWrong', 'InboxLookup.Wrong.rejects_same_full_contract'),
}
THEOREM_TYPE = 'SandboxModel.Stage1Protocol.InboxLookup.FullLookupContract '
LOOKUP = 'SandproofStage1RequestRust.sandproof_controller_service.controller.lifecycle_inbox.model.Inbox.find'
FREEZE = 'verification/stage1-protocol-inputs.sha256'
WRONG_PAGE = str(HERE / 'InboxLookupWrong.lean.in')
WRONG_FREEZES = {FREEZE, 'verification/protocol-convergence-inputs.sha256'}
MODEL_PAGE = str(HERE / 'InboxLookupModel.lean.in')
LOCAL_LOOKUP_PAGES = {str(HERE / 'InboxLookup.lean.in'), MODEL_PAGE}
LOCAL_LOOKUP_FREEZES = {*WRONG_FREEZES, 'verification/create-delivery-inputs.sha256',
                       'verification/v3-transaction-source-inputs.sha256'}
NATIVE_RECEIPT = 'lookup-native-receipt.json'
NATIVE_VARIANTS = ('original', 'candidate', 'key-only', 'restored')
NATIVE_COMMANDS = {
    'test': ['--lib', '--message-format=json', 'proof_lookup::', '--', '--nocapture'],
    'clippy': ['--lib', '--tests', '--', '-D', 'warnings'],
}
COMPILER_OVERRIDE = re.compile(r'(?:(?:CARGO_BUILD_)?(?:RUSTC(?:_WORKSPACE_WRAPPER|_WRAPPER)?|RUSTDOC)'
                               r'|(?:HOST_|TARGET_)?(?:CC|CXX|AR|LD)(?:_[a-zA-Z0-9_-]+)?'
                               r'|CARGO_TARGET_[A-Z0-9_]+_LINKER)')
LINKER_FLAGS = re.compile(r'(?:^|[\s\x1f])-C\s*(?:linker(?:-flavor)?|link-args?)(?=[=\s\x1f]|$)')
METADATA_FILES = {'inventory.json', 'frozen.json', 'candidate.json', 'package-inputs.json',
                  'sources.json', 'input-closure.json'}

sys.path.insert(0, str(ROOT))
from tools.acceptance_receipts import (contract_inputs, digest, native_test_configuration,
                                       repository_inputs, rules, rust_inputs, sha, tree_digest, write_json)


def read(path):
    return json.loads(path.read_text())


def git(*args, root=ROOT):
    return subprocess.check_output(['git', *args], cwd=root)


def clean_candidate(root=ROOT):
    if git('status', '--porcelain', '-z', '--untracked-files=normal', root=root):
        raise ValueError('module acceptance requires a clean committed checkout')
    return {
        'commit': git('rev-parse', 'HEAD', root=root).decode().strip(),
        'tree': git('rev-parse', 'HEAD^{tree}', root=root).decode().strip(),
    }


def changed_files(base, candidate, root=ROOT):
    base = git('rev-parse', '--verify', '--end-of-options', base + '^{commit}', root=root).decode().strip()
    if subprocess.run(['git', 'merge-base', '--is-ancestor', base, candidate], cwd=root).returncode != 0:
        raise ValueError('the comparison base must be an ancestor of the candidate')
    paths = git('diff', '--name-only', '--no-renames', '-z', base, candidate, '--', root=root)
    return base, [os.fsdecode(path) for path in paths.split(b'\0') if path]


def downstream(root, seed_templates):
    """Find concrete reverse imports/copy references for the review report.

    This scan is not an independence proof. Shared or unknown paths always
    require wider review, even if the scan finds no additional consumers.
    """
    stage = root / STAGE1
    owners = defaultdict(set)
    templates = sorted(stage.rglob('*.lean.in'))
    for path in templates:
        owner = path.relative_to(stage).parts[0]
        owners[path.name.removesuffix('.lean.in')].add(owner)
    reverse = defaultdict(set)
    for path in templates:
        owner = path.relative_to(stage).parts[0]
        imports = re.findall(r'^\s*(?:public )?import\s+(\S+)', path.read_text(), re.M)
        for name in imports:
            stem = name.rsplit('.', 1)[-1]
            providers = owners[stem]
            # Types/Funs are repeated for separate physical views; their names
            # alone cannot identify which physical provider a driver uses.
            if len(providers) != 1:
                continue
            provider, = providers
            if provider != owner and (provider != MODULE or stem in seed_templates):
                reverse[provider].add(owner)

    references = re.compile(r"['\"/]([a-z][a-z0-9]*(?:-[a-z0-9]+)+)(?=['\"/])")
    directories = {path.name for path in stage.iterdir() if path.is_dir()}
    for path in sorted(stage.rglob('*.py')):
        parts = path.relative_to(stage).parts
        if len(parts) < 2:
            continue
        owner = parts[0]
        text = path.read_text()
        for provider in (set(references.findall(text)) & directories) - {owner}:
            if provider == MODULE and seed_templates == {'InboxLookupWrong'} and 'InboxLookupWrong' not in text:
                continue
            reverse[provider].add(owner)
    direct = reverse[MODULE]
    affected = set(direct)
    pending = list(direct)
    while pending:
        for name in reverse[pending.pop()] - affected - {MODULE}:
            affected.add(name)
            pending.append(name)
    return sorted(direct), sorted(affected)


def select(paths, root=ROOT):
    inventory = read(root / HERE / 'inventory.json')
    paths = sorted(set(paths))
    scope = dict(changed_paths=paths, affected_modules=[], checks=[], direct_consumers=[])
    proof_paths = set(paths) - {WORKFLOW_DOC}
    if not proof_paths:
        return dict(scope, status='empty', reason='no changed inputs; no module was accepted')
    if proof_paths <= ISOLATED_INPUTS:
        # If even the isolated mutant template gets a new downstream reader,
        # stop treating it as isolated. Other inputs here are pilot tooling.
        if WRONG_PAGE not in paths or not (lean_consumers(root, {WRONG_PAGE}) - {MODULE}):
            return dict(scope, status='ready', affected_modules=[MODULE],
                        checks=['original-native', 'formal-correct', 'formal-key-only'],
                        production_entry=f'{MODEL}:Inbox::find / Session::find_entry',
                        rust_test_filter='sandproof-controller-service: proof_lookup::',
                        extraction_roots=str(HERE / 'formal.py') + ':ROOTS',
                        input_inventory=str(HERE / 'inventory.json'),
                        contract=THEOREM_TYPE.strip())
    shared_modules = set(inventory['production_inputs']) | {
        'Cargo.lock', 'Cargo.toml', 'lean-toolchain', 'rust-toolchain.toml', 'lakefile.toml',
        'tools/proof_batch.py', 'tools/proof_services.py', 'tools/acceptance_receipts.py',
        str(STAGE1 / 'formal_runtime.py'), str(STAGE1 / 'registered.py'),
        'verification/foundations-environment.json', 'verification/stage1-protocol-inputs.sha256',
    }
    shared = [path for path in paths if path in shared_modules or Path(path).is_relative_to(HERE)]
    unknown = sorted(proof_paths - set(shared) - ISOLATED_INPUTS)
    direct, affected = downstream(root, set(inventory['audits']))
    return dict(scope, status='review-required', affected_modules=[MODULE, *affected],
        direct_consumers=direct, shared_inputs=shared, unmapped_inputs=unknown,
        reason='use the registered drivers for the reviewed dependency set or run full Stage1 at promotion; no module pass was issued')


def registration(root):
    spec = importlib.util.spec_from_file_location('lookup_registered_stage1', root / STAGE1 / 'registered.py')
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def refreshed_inputs(value, hashes):
    if isinstance(value, list):
        return [refreshed_inputs(row, hashes) for row in value]
    if isinstance(value, dict):
        revised = {key: (hashes[key] if key in hashes and isinstance(row, str)
                         and re.fullmatch('[0-9a-f]{64}', row) else refreshed_inputs(row, hashes))
                   for key, row in value.items()}
        repository = value.get('repository')
        if (isinstance(repository, str) and repository in hashes
                and isinstance(value.get('sha256'), str) and re.fullmatch('[0-9a-f]{64}', value['sha256'])):
            revised['sha256'] = hashes[repository]
        return revised
    return value


def reviewed_refresh(root, base, name, hashes):
    """Accept only an exact source-digest synchronization, never a new rule."""
    if not (root / name).is_file():
        return False
    old = git('show', f'{base}:{name}', root=root)
    if name in LOCAL_LOOKUP_FREEZES:
        revised = re.sub(r'^([0-9a-f]{64})(  )([^\n]+)$',
                         lambda m: hashes.get(m[3], m[1]) + m[2] + m[3], old.decode(), flags=re.M)
        return revised != old.decode() and revised == (root / name).read_text()
    before = json.loads(old)
    revised = refreshed_inputs(before, hashes)
    return revised != before and revised == read(root / name)


def lean_consumers(root, seeds):
    """Follow proof pages before grouping them into original registered jobs."""
    stage = root / STAGE1
    modules = {Path(name).name.removesuffix('.lean.in') for name in seeds}
    templates = {}
    for path in sorted(stage.rglob('*.lean.in')):
        lines = re.findall(r'^\s*(?:(?:public|private) )?import[ \t]+([^\r\n]+)', path.read_text(), re.M)
        templates[path] = {name.rsplit('.', 1)[-1] for line in lines for name in line.split()}
    seen = set()
    while True:
        reached = {path for path, imports in templates.items() if imports & modules} - seen
        if not reached:
            break
        seen.update(reached)
        modules.update(path.name.removesuffix('.lean.in') for path in reached)
    # Original drivers copy leaves into separate physical projects. Include
    # their explicit template names and any names in frozen per-view recipes.
    tokens = '|'.join(re.escape(name) for name in sorted(modules))
    mentions = re.compile(r'(?<![\w])(?:' + tokens + r')(?=\.lean|[\'\"])')
    for path in sorted(stage.rglob('*')):
        if (path.parent != stage and (path.suffix == '.py' or path.name in METADATA_FILES)
                and path != root / HERE / 'module_acceptance.py' and mentions.search(path.read_text())):
            seen.add(path)
    consumers = {path.relative_to(stage).parts[0] for path in seen}
    for directory in ('SandboxModel', 'verification/aeneas'):
        for path in (root / directory).rglob('*.lean'):
            imports = re.findall(r'^\s*(?:(?:public|private) )?import[ \t]+([^\r\n]+)', path.read_text(), re.M)
            if any(name.rsplit('.', 1)[-1] in modules for line in imports for name in line.split()):
                consumers.add(str(path.relative_to(root)))
    return consumers


def native_reuse_paths(paths, base, root=ROOT):
    """The one reviewed Lean page and its deterministic checksum updates."""
    if set(paths) != {WRONG_PAGE, *WRONG_FREEZES}:
        raise ValueError('native reuse requires only InboxLookupWrong.lean.in and its two exact freeze updates')
    for name in paths:
        if (root / name).is_symlink() or not (root / name).is_file():
            raise ValueError('native reuse requires regular tracked inputs: ' + name)
    hashes = {WRONG_PAGE: sha(root / WRONG_PAGE)}
    if any(not reviewed_refresh(root, base, name, hashes) for name in WRONG_FREEZES):
        raise ValueError('freeze edit is not the exact checksum refresh for InboxLookupWrong.lean.in')
    if lean_consumers(root, {WRONG_PAGE}) - {MODULE}:
        raise ValueError('InboxLookupWrong.lean.in has an unreviewed downstream consumer')
    return sorted(paths)


def native_argv(label):
    return ['cargo', label, '-p', 'sandproof-controller-service', '--locked', '--offline', *NATIVE_COMMANDS[label]]


def check_native_tool_overrides(environment, configurations, pinned):
    """Reuse only compiler tools independently fingerprinted by this native key."""
    def tool(name, value, defaults=()):
        if value:
            simple = isinstance(value, str) and (os.path.isabs(value) or '/' not in value)
            selected = shutil.which(value, path=environment.get('PATH')) if simple else None
            if selected is None or Path(selected).resolve() not in pinned_paths(defaults):
                raise ValueError('unregistered native compiler or linker override: ' + name)

    def pinned_paths(names):
        return {path for name in names for path in pinned.get(name, ())}

    def env_tool(name, setting):
        value = setting.get('value') if isinstance(setting, dict) else setting
        if COMPILER_OVERRIDE.fullmatch(name):
            if value and isinstance(setting, dict) and setting.get('relative'):
                raise ValueError('unregistered native compiler or linker override: ' + name)
            if name in ('RUSTC', 'CARGO_BUILD_RUSTC'):
                defaults = ('rustc',)
            elif name.endswith('_LINKER'):
                defaults = ('cc', 'ld')
            else:
                args = re.fullmatch(r'(?:HOST_|TARGET_)?(CC|AR|LD)(?:_[a-zA-Z0-9_-]+)?', name)
                defaults = (args[1].lower(),) if args else ()
            tool(name, value, defaults)
        if name.endswith('RUSTFLAGS'):
            flags(name, value)

    def flags(name, value):
        text = ' '.join(value) if isinstance(value, list) else value
        if text and LINKER_FLAGS.search(text):
            raise ValueError('unregistered native linker override: ' + name)

    for name, value in environment.items():
        env_tool(name, value)
    for path in configurations:
        options = tomllib.loads(path.read_text())
        for name, value in options.get('build', {}).items():
            if name in ('rustc', 'rustdoc', 'rustc-wrapper', 'rustc-workspace-wrapper'):
                tool(f'{path}:build.{name}', value, ('rustc',) if name == 'rustc' else ())
            elif name == 'rustflags':
                flags(f'{path}:build.{name}', value)
        for target, value in options.get('target', {}).items():
            tool(f'{path}:target.{target}.linker', value.get('linker'), ('cc', 'ld'))
            flags(f'{path}:target.{target}.rustflags', value.get('rustflags'))
        for name, value in options.get('env', {}).items():
            env_tool(name, value)


def native_component_key(root=ROOT, *, scoped_output=None):
    """Only a live invocation can capture this standalone native input key."""
    tools, pinned = {}, {}
    for name in ('cargo', 'rustup', 'rustc', 'cc', 'ld', 'ar'):
        path = shutil.which(name)
        if path is None:
            raise ValueError('missing native command tool: ' + name)
        actual = Path(path).resolve(strict=True)
        tools[name] = sha(actual)
        pinned[name] = {actual}
    for name, argv in [('cargo', ['-V']), ('rustc', ['-Vv']), ('clippy-driver', ['--version'])]:
        binary = Path(subprocess.check_output(['rustup', 'which', name], cwd=root, text=True).strip())
        tools['rustup:' + name] = dict(sha256=sha(binary),
            version=subprocess.check_output([str(binary), *argv], cwd=root, text=True))
        pinned.setdefault(name, set()).add(binary.resolve(strict=True))
    sysroot = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], cwd=root, text=True).strip())
    cargo = Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')))
    cargo_config = {str(p): sha(p) for p in (cargo/'config', cargo/'config.toml') if p.is_file()}
    # Values are bound, never published. These process/session placeholders
    # do not enter the Rust driver, compiler, or its assertion configuration.
    runtime_env = {k: v for k, v in os.environ.items()
                   if k not in {'PWD', 'OLDPWD', 'SHLVL', '_', 'SSH_CLIENT', 'SSH_CONNECTION', 'SSH_TTY'}}
    rust = rust_inputs(root)
    if scoped_output is not None:
        configs = {Path(p) for p in cargo_config}
        configs.update(Path(p.removeprefix('@configuration:')) for p in rust
                       if p.startswith('@configuration:') and Path(p).name in ('config', 'config.toml'))
        configs.update(p for parent in (scoped_output, *scoped_output.parents)
                       for suffix in ('config', 'config.toml') if (p := parent/'.cargo'/suffix).is_file())
        check_native_tool_overrides(os.environ, sorted(configs), pinned)
    local_config = '@configuration:' + str(root) + '/'
    rust = {('@configuration:$SOURCE/' + k.removeprefix(local_config) if k.startswith(local_config) else k): v
            for k, v in rust.items()}
    modes = {}
    for entry in git('ls-files', '--stage', '-z', root=root).decode().split('\0'):
        if entry:
            metadata, name = entry.split('\t', 1)
            mode, _, stage = metadata.split()
            path = root / name
            if (stage != '0' or mode not in ('100644', '100755') or path.is_symlink() or not path.is_file()
                    or bool(path.stat().st_mode & 0o100) != (mode == '100755')):
                raise ValueError('invalid original native source kind or mode: ' + name)
            modes[name] = mode
    return dict(kind='inbox-lookup-native-key-v1', scoped_native_tools=scoped_output is not None,
                source=repository_inputs(root), modes=modes,
                rust=rust, rules=rules(root), contracts=contract_inputs(root),
                runtime=dict(tools=tools, python_sha256=sha(Path(sys.executable).resolve()),
                             environment_sha256=digest(runtime_env), test_configuration=native_test_configuration(os.environ),
                             rust_libraries=tree_digest(sysroot/'lib'),
                             cargo_config=cargo_config,
                             platform=sys.platform, uid=os.getuid(), gid=os.getgid()),
                commands={label: native_argv(label) for label in NATIVE_COMMANDS})


def compare_native_keys(original, current, permitted):
    if (original.get('kind') != 'inbox-lookup-native-key-v1' or original.keys() != current.keys()
            or proposed_changes(original['source'], current['source']) != set(permitted)):
        raise ValueError('original native key does not bind the exact documented proof-only change')
    if original.get('scoped_native_tools') is not True or current.get('scoped_native_tools') is not True:
        raise ValueError('standalone native reuse requires a scoped tool-checked original preparation')
    for field in original:
        before, after = original[field], current[field]
        if field in ('source', 'rules'):
            if before.keys() != after.keys() or not proposed_changes(before, after) <= set(permitted):
                raise ValueError('undeclared native input changed: ' + field)
        elif before != after:
            raise ValueError('native key changed: ' + field)


def proposed_changes(before, after):
    return {name for name in before.keys() | after.keys() if before.get(name) != after.get(name)}


def checked_proof_inputs(paths, seeds, freezes, root, base, ignored=()):
    """Bind proof seeds and only their exact derived checksum edits."""
    hashes = {name: sha(root / name) for name in seeds if (root / name).is_file()}
    if set(hashes) != seeds:
        return None
    pending = set(paths) - seeds - set(ignored) - freezes
    if any(base is None or not Path(name).is_relative_to(STAGE1) or Path(name).name not in METADATA_FILES
           for name in pending):
        return None
    while pending:
        refreshed = {name for name in pending if reviewed_refresh(root, base, name, hashes)}
        if not refreshed:
            return None
        hashes.update({name: sha(root / name) for name in refreshed})
        pending -= refreshed
    if any(base is None or not reviewed_refresh(root, base, name, hashes) for name in set(paths) & freezes):
        return None
    return hashes


def check_original_lookup_model(root, base):
    # These are the actual definitions referenced by the common proof and its
    # counterexample. A dependency-only edit can change their proof lemmas,
    # while the original imports, definitions and contract remain identical.
    boundary = b'\ntheorem entry_absent_iff ('
    old = git('show', f'{base}:{MODEL_PAGE}', root=root)
    current = (root / MODEL_PAGE).read_bytes()
    if (old.count(boundary) != 1 or current.count(boundary) != 1
            or old.split(boundary, 1)[0] != current.split(boundary, 1)[0]):
        raise ValueError('scoped Lookup reuse requires the original shared Lean model definitions')


def local_lookup_impact(paths, base, root=ROOT):
    seeds = set(paths) & LOCAL_LOOKUP_PAGES
    if not seeds or checked_proof_inputs(paths, seeds, LOCAL_LOOKUP_FREEZES, root, base) is None:
        raise ValueError('Lookup-only reuse requires positive/shared proof pages and exact derived checksum edits')
    if MODEL_PAGE in seeds:
        check_original_lookup_model(root, base)
    impact = selected_registered_impact(seeds, root)
    local = {f'{MODULE}-{variant}' for variant in VARIANTS}
    if not local <= {job['name'] for job in impact['required_jobs']}:
        raise ValueError('missing original Lookup positive or mutant job')
    return dict(impact, pending_jobs=[job for job in impact['required_jobs'] if job['name'] not in local])


def registered_impact(paths, root=ROOT, base=None):
    """Resolve one reviewed business Rust file or original lookup Lean pages
    to concrete registered jobs, keeping every original native prerequisite.
    """
    inventory = read(root / HERE / 'inventory.json')
    pages = {str(HERE / (name + '.lean.in')) for name in inventory['audits']}
    seeds = set(paths) & (pages | {MODEL})
    if not seeds:
        return None
    if checked_proof_inputs(paths, seeds, {FREEZE}, root, base, ignored=ISOLATED_INPUTS) is None:
        return None
    return selected_registered_impact(seeds, root)


def selected_registered_impact(seeds, root):
    inventory = read(root / HERE / 'inventory.json')
    pages = {str(HERE / (name + '.lean.in')) for name in inventory['audits']}
    stage = root / STAGE1
    actual_sources = []
    affected = {MODULE, *lean_consumers(root, seeds & pages)} if MODEL not in seeds else {
        MODULE, *downstream(root, set(inventory['audits']))[1]}
    if MODEL in seeds:
        # A file is the current registered Rust freeze unit. This file also
        # implements recording, validation, and retention, not lookup alone.
        for path in sorted(stage.rglob('*.json')):
            if path.name not in METADATA_FILES:
                continue
            value = read(path)
            if value != refreshed_inputs(value, {MODEL: ''}):
                relative = path.relative_to(stage)
                actual_sources.append(str(path.relative_to(root)))
                affected.add(relative.parts[0] if len(relative.parts) > 1 else 'stage1-protocol')

    registry = registration(root)
    aliases = {'inbox-append': 'inbox-record-total', 'source-live': 'source-live',
               'unload-parent-link': registry.UNLOAD.PARENT_LABEL,
               **dict.fromkeys(registry.UNLOAD.PACKAGES, registry.UNLOAD.LABEL)}
    jobs_by_module = defaultdict(list)
    for job in registry.formal_plan(Path('/unused'), root=root):
        directory = Path(job['driver']).parent.name
        # Root-level and external drivers serve independent roles. Map such
        # consumers by their registered names, not the common driver folder.
        if directory in ('stage1-protocol', 'tools'):
            directory = job['name']
        jobs_by_module[directory].append(job)
    modules = {name for name in affected if name != 'stage1-protocol'}
    unmapped = sorted(name for name in modules if aliases.get(name, name) not in jobs_by_module)
    if 'stage1-protocol' in affected:
        unmapped.append('stage1-protocol-retained')
    selected = {job['name']: job for name in modules
                for job in jobs_by_module[aliases.get(name, name)]}
    jobs = [dict(name=job['name'], driver=job['driver'], args=job['args'],
                 native=job['native'], cases=job['cases'])
            for job in sorted(selected.values(), key=lambda job: job['name'])]
    rust = []
    if MODEL in seeds:
        rust = [
            ['cargo', '--config', 'scripts/check-profiles.toml', 'test', '--locked', '--offline',
             '-p', 'sandproof-controller-service', '--lib', 'controller::lifecycle_inbox::tests::'],
            ['cargo', '--config', 'scripts/check-profiles.toml', 'clippy', '--locked', '--offline',
             '-p', 'sandproof-controller-service', '--all-targets', '--', '-D', 'warnings'],
        ]
    native_labels = {'functional', *(label for job in jobs for label in job['native'])}
    return dict(seed_inputs=sorted(seeds), affected_modules=sorted(affected), required_jobs=jobs, unmapped=unmapped,
                related_rust_checks=rust, registered_rust_inputs=actual_sources,
                original_native_prerequisites=sorted(native_labels),
                native_scope='stage1-protocol',
                native_command=['bash', 'scripts/python-entry.sh', 'tools/proof_native_suite.py', 'run',
                                '--scope', 'stage1-protocol', '--output', '<current-native-evidence>'],
                native_slice_limitation=('original native rules include tracked proof templates; functional also '
                                         'binds every tracked repository input. Stale seals are rejected.'),
                executor='--output <new-private-dir> --formal-lock <operator-lock> --registered-native <current-native-evidence>')


def run_driver(output, tool, *args):
    target = output / tool
    name = 'native.py' if tool == 'original-native' else 'formal.py'
    log = output / (tool + '.log')
    argv = [sys.executable, str(ROOT / HERE / name), '--output', str(target), *args]
    with log.open('xb') as stream:
        result = subprocess.run(argv, cwd=ROOT, stdin=subprocess.DEVNULL,
                                stdout=stream, stderr=subprocess.STDOUT)
    if result.returncode:
        raise ValueError(f'{tool} failed with exit {result.returncode}; see {log}')
    return read(target / 'result.json')


def admit_native(record, folder, commit):
    expected = set(NATIVE_VARIANTS)
    if (record['kind'] != 'inbox-lookup-native-development' or record['complete'] is not True
            or record['source_commit'] != commit or set(record['variants']) != expected
            or any(row['complete'] is not True for row in record['variants'].values())):
        raise ValueError('missing current complete original Rust views')
    seen = set()
    for row in record['commands']:
        variant, label = key = (row['variant'], row['label'])
        mutant = key == ('key-only', 'test')
        log = folder / variant / (label + '.log')
        if (variant not in expected or label not in ('test', 'clippy') or key in seen
                or row['argv'] != native_argv(label)
                or type(row['exit_code']) is not int or row['exit_code'] != (101 if mutant else 0)
                or row.get('property_rejected', False) is not mutant
                or log.is_symlink() or log.parent.is_symlink() or not log.is_file()
                or sha(log) != row['log_sha256']):
            raise ValueError('invalid original Rust command evidence: ' + repr(key))
        seen.add(key)
    if len(seen) != 2 * len(expected):
        raise ValueError('missing original Rust test or Clippy evidence')


def checked_native_payload(receipt, directory, candidate):
    folder = directory / 'original-native'
    result_path = folder / 'result.json'
    launcher_log = directory / 'original-native.log'
    for path, field in ((result_path, 'result_sha256'), (launcher_log, 'launcher_log_sha256')):
        if folder.is_symlink() or path.is_symlink() or not path.is_file() or sha(path) != receipt[field]:
            raise ValueError('missing or changed original native result or launcher log: ' + str(path))
    native = read(result_path)
    admit_native(native, folder, candidate['commit'])
    key = receipt['key']
    if (native['inputs'] != key['source'] or receipt['native_registry_sha256'] != digest(native['registry'])
            or any({field: native['tools'][name][field] for field in ('sha256', 'version')}
                   != key['runtime']['tools']['rustup:' + name]
                   for name in ('cargo', 'rustc', 'clippy-driver'))):
        raise ValueError('original native case does not bind its captured key')
    return native


def publish_native_receipt(output, candidate, key, native):
    receipt = dict(schema=1, kind='inbox-lookup-standalone-native-receipt', candidate=candidate,
                   key=key, result_sha256=sha(output/'original-native/result.json'),
                   launcher_log_sha256=sha(output/'original-native.log'),
                   native_registry_sha256=digest(native['registry']))
    checked_native_payload(receipt, output, candidate)
    path = output / NATIVE_RECEIPT
    write_json(path, receipt)
    return dict(use='RUN', seconds=native['seconds'], result_sha256=receipt['result_sha256'],
                receipt=str(path), receipt_sha256=sha(path))


def load_native_reuse(path, expected_sha, base, key, permitted):
    if (path.name != NATIVE_RECEIPT or path.is_symlink() or not path.is_file()
            or not re.fullmatch('[0-9a-f]{64}', expected_sha) or sha(path) != expected_sha):
        raise ValueError('missing or changed standalone original native receipt')
    receipt = read(path)
    if (receipt.get('schema') != 1 or receipt.get('kind') != 'inbox-lookup-standalone-native-receipt'
            or receipt.get('candidate') != base):
        raise ValueError('native proof-only reuse requires its exact original candidate receipt')
    compare_native_keys(receipt['key'], key, permitted)
    native = checked_native_payload(receipt, path.parent, base)
    return native, dict(use='REUSE', seconds=0, original_seconds=native['seconds'],
                        result_sha256=receipt['result_sha256'], receipt=str(path), receipt_sha256=expected_sha,
                        original_candidate=base, source_differences=permitted)


def admit_scoped_lookup_native(binding, native, inputs, registry, permitted, root=ROOT):
    """Read the original native snapshot separately from this proof projection."""
    from native_admission import registry_identity

    if not permitted or proposed_changes(native['inputs'], inputs) != set(permitted):
        raise ValueError('formal source differs from original native beyond the admitted proof pages and checksums')
    lock = root / HERE / 'Cargo.lock'
    if sha(lock) != inputs[str(HERE/'Cargo.lock')]:
        raise ValueError('registry Cargo.lock differs from admitted source input')
    registry_identity.require_same_content(native['registry'], registry, tomllib.loads(lock.read_text()))
    binding.update(native_inputs_match=True, registry_content_match=True)


def admit_formal(native, record, folder, variant, commit, permitted=()):
    # Apply the same original component source and registry admission as the
    # full driver, with the narrow original-native record created in this run.
    from native_admission import admit

    rust_variant, module, theorem = VARIANTS[variant]
    if (record['kind'] != 'inbox-lookup-formal-development' or record['complete'] is not True
            or record['source_commit'] != commit or record['variant'] != variant):
        raise ValueError('missing current complete original Lean view: ' + variant)
    binding = {}
    if permitted:
        admit_scoped_lookup_native(binding, native, record['inputs'], record['registry'], permitted)
    else:
        admit(binding, native, record['inputs'], record['registry'], ROOT, HERE / 'Cargo.lock')
    source_hash = record['candidate_model_sha256']
    source = folder / 'source'
    if (native['variants'][rust_variant]['inputs'][MODEL] != source_hash
            or sha(source / MODEL) != source_hash
            or native['variants']['original']['inputs'][MODEL] != native['variants']['restored']['inputs'][MODEL]):
        raise ValueError('Rust and Lean views do not bind the same lookup implementation')
    for name in ('cargo', 'rustc'):
        if native['tools'][name]['sha256'] != record['runtime']['tools'][name]['sha256']:
            raise ValueError('Rust and Lean driver tool differs: ' + name)
    full_theorem = 'SandboxModel.Stage1Protocol.' + theorem
    proof_page = f'SandboxModel/Lifecycle/Protocol/{module}.lean'
    actual = record['actual']
    if (full_theorem not in actual['audits'] or sha(source / proof_page) != actual['files'][proof_page]
            or sha(folder / 'physical-files/service.log') != actual['service_log_sha256']):
        raise ValueError('missing original full lookup theorem, physical proof, or service evidence')
    if set(record['llbc']) != {'original', 'candidate'}:
        raise ValueError('missing original or candidate LLBC')
    for ir in record['llbc'].values():
        if sha(source / ir['path']) != ir['sha256']:
            raise ValueError('LLBC evidence changed')
    return dict(theorem=full_theorem, property_rejected=variant == 'key-only',
                rust_formal_inputs_match=binding['native_inputs_match'],
                rust_formal_documented_source_differences=list(permitted),
                registry_content_match=binding['registry_content_match'],
                candidate_model_sha256=source_hash, seconds=record['seconds'],
                result_sha256=sha(folder / 'result.json'), **check_contract_type(folder, variant))


def check_contract_type(folder, variant, evidence=None):
    # Audited declaration names alone do not give the theorem's Lean type.
    # Check the precise proposition even when the mutant proof template is edited.
    _, module, theorem = VARIANTS[variant]
    full_theorem = 'SandboxModel.Stage1Protocol.' + theorem
    source = folder / 'source'
    negation = '¬ ' if variant == 'key-only' else ''
    proposition = negation + THEOREM_TYPE + LOOKUP
    if evidence is not None:
        evidence.mkdir(mode=0o700, parents=True)
    check = (evidence if evidence is not None else source / 'artifacts') / 'ModuleAcceptance.lean'
    check.write_text(f'import SandboxModel.Lifecycle.Protocol.{module}\n'
                     f'#check ({full_theorem} : {proposition})\n')
    log = (evidence if evidence is not None else folder) / 'contract-type.log'
    args = ['lake', 'env', 'lean']
    if evidence is not None:
        args += ['--root=' + str(check.parent)]
    with log.open('xb') as stream:
        checked = subprocess.run([*args, str(check)], cwd=source,
                                 stdin=subprocess.DEVNULL, stdout=stream,
                                 stderr=subprocess.STDOUT, timeout=120)
    if checked.returncode != 0:
        raise ValueError('wrong full lookup theorem type; see ' + str(log))
    return dict(contract_type_log_sha256=sha(log), contract_type_source_sha256=sha(check))


def prepare_output(output, reuse=None):
    output = output.resolve()
    if output.is_relative_to(ROOT) or (reuse is not None and output.is_relative_to(reuse[0].parent.resolve())):
        raise ValueError('use a fresh private output and caches outside the source checkout')
    output.mkdir(mode=0o700)
    return output


def check_lookup_sources():
    inventory = read(ROOT / HERE / 'inventory.json')
    for name, digest in inventory['production_inputs'].items():
        if sha(ROOT / name) != digest:
            raise ValueError('reviewed Inbox Lookup input differs before running checks: ' + name)


def prepare_native(output, candidate, *, scoped=False):
    native_output = output if scoped else None
    key = native_component_key(scoped_output=native_output)
    native = run_driver(output, 'original-native')
    if clean_candidate() != candidate or native_component_key(scoped_output=native_output) != key:
        raise ValueError('native inputs, tools or environment changed during execution')
    return native, publish_native_receipt(output, candidate, key, native)


def execute_native_preparation(output, candidate):
    output = prepare_output(output)
    started = time.monotonic()
    result = dict(kind='inbox-lookup-native-preparation', candidate=candidate, status='failed')
    try:
        check_lookup_sources()
        _, result['native'] = prepare_native(output, candidate, scoped=True)
        result['status'] = 'passed'
    except Exception as exc:
        result['error'] = str(exc)
    result['seconds'] = time.monotonic() - started
    write_json(output / 'native-preparation.json', result)
    print(json.dumps(result, ensure_ascii=False))
    return 0 if result['status'] == 'passed' else 1


def execute(output, plan, candidate, formal_lock, reuse=None, *, lookup_only=False):
    output = prepare_output(output, reuse)
    started = time.monotonic()
    report = dict(schema=1, kind='stage1-module-acceptance', module=MODULE, complete=False,
                  candidate=candidate, plan=plan, fresh_runs=reuse is None, full_stage1_acceptance=False,
                  host=platform.node(), native_test_configuration=native_test_configuration(os.environ),
                  input_inventory_sha256=sha(ROOT / HERE / 'inventory.json'), formal={})
    if hasattr(os, 'sched_getaffinity'):
        report['available_cpus'] = sorted(os.sched_getaffinity(0))
    try:
        check_lookup_sources()
        from formal import execution
        execution.tool_bind(ROOT)
        permitted = plan.get('native_reuse_paths', [])
        if reuse is None:
            native, report['native'] = prepare_native(output, candidate)
        else:
            base = dict(commit=plan['base_commit'], tree=git('rev-parse', plan['base_commit']+'^{tree}').decode().strip())
            native, report['native'] = load_native_reuse(*reuse, base, native_component_key(scoped_output=output), permitted)
            from native import registry_module
            if registry_module.registry() != native['registry']:
                raise ValueError('current verified locked registry differs from the original native receipt')
        waiting = time.monotonic()
        print('Original Rust views admitted; waiting for the existing formal lock.', flush=True)
        with formal_lock.open('rb') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            report['formal_lock_wait_seconds'] = time.monotonic() - waiting
            for variant in VARIANTS:
                phase = 'formal-' + variant
                result = run_driver(output, phase, '--variant', variant)
                report['formal'][variant] = admit_formal(native, result, output / phase, variant, candidate['commit'], permitted)
        if reuse is not None:
            load_native_reuse(*reuse, base, native_component_key(scoped_output=output), permitted)
        if clean_candidate() != candidate or native_test_configuration(os.environ) != report['native_test_configuration']:
            raise ValueError('candidate or native test configuration changed during checking')
        if lookup_only:
            report['lookup_only_complete'] = True
        else:
            report['complete'] = True
    except Exception as exc:
        report['error'] = str(exc)
    report['seconds'] = time.monotonic() - started
    if lookup_only:
        report['kind'] = 'inbox-lookup-only-proof'
        write_json(output / 'lookup-local-result.json', report)
    else:
        write_json(output / 'module-result.json', report)
    print(json.dumps(report, ensure_ascii=False))
    return 2 if report.get('lookup_only_complete') else (0 if report['complete'] else 1)


def execute_registered(output, plan, candidate, formal_lock, native):
    """Run the original registered subset against the original same-tree seal."""
    registered = registration(ROOT)
    shared = registered._shared()
    impact = plan['registered_impact']
    if impact['unmapped'] or not impact['required_jobs']:
        raise ValueError('required consumer has no original registered job; use complete Stage1')
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('registered execution requires the original formal role')
    native = native.resolve(strict=True)
    output = output.resolve()
    if output.is_relative_to(ROOT) or output.is_relative_to(native) or native.is_relative_to(output):
        raise ValueError('use a private output separate from the candidate and native evidence')
    directory = output / 'registered'
    job_names = {row['name'] for row in impact['required_jobs']}
    jobs = [job for job in registered.formal_plan(directory, root=ROOT) if job['name'] in job_names]
    if {job['name'] for job in jobs} != job_names:
        raise ValueError('a selected original job is absent from the current registry')
    output.mkdir(mode=0o700)
    report = dict(kind='stage1-module-registered-acceptance', candidate=candidate, plan=plan,
                  complete=False, full_stage1_acceptance=False, results=[])
    started = time.monotonic()
    try:
        os.environ['SANDPROOF_NATIVE_EVIDENCE'] = str(native)
        registered.validate_registration(ROOT, profile='full')
        shared.validate_native(native, scope='stage1-protocol')
        receipts = {label: shared.native_case(label, ROOT, None, scope='stage1-protocol')
                    for label in impact['original_native_prerequisites']}
        for label in receipts.keys() - {'functional'}:
            folder = registered._original_file(receipts[label], 'result.json').parent
            for create_label, driver, _ in registered.CREATE_NATIVE_DRIVERS:
                if label == create_label:
                    registered._check_create_native_result(ROOT, Path(driver).parent.name,
                                                           folder, read(folder / 'result.json'))
        paths = {name: registered._evidence(receipt) for name, receipt in receipts.items()}
        directory.mkdir(mode=0o700)
        with formal_lock.open('r+b') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            os.environ.update(SANDPROOF_LOCK_FILE=str(formal_lock.resolve()),
                              SANDPROOF_LOCK_FD=str(lock.fileno()))
            registered.check_create_sources(ROOT)
            helper = registered._execution_helper(ROOT)
            context = registered._execution_context(ROOT, shared, helper, receipts)
            report['native_seal_sha256'] = context['native']['seal_sha256']

            def source_guard():
                if (clean_candidate() != candidate or sha(native / 'complete.json') != report['native_seal_sha256']
                        or shared.repository_inputs(ROOT) != context['repository']['files']
                        or registered._source_modes(ROOT) != context['repository']['modes']):
                    raise ValueError('candidate, repository inputs or original native seal changed')
                registered.check_create_sources(ROOT)

            options = registered._execution_options()
            rows, failures = registered.execute_formal_jobs(
                ROOT, directory, jobs, context, paths, jobs=options['workers'], source_guard=source_guard,
                runtime_seed=Path(os.environ['SANDPROOF_FORMAL_CACHE']) / 'runtime-file-hashes.json')
            report.update(results=rows, failures=failures)
            if failures or [row['name'] for row in rows] != [job['name'] for job in jobs]:
                raise ValueError('one or more selected original proof jobs did not produce an admitted checkpoint')
            report['lookup_contract_types'] = {
                variant: check_contract_type(directory / f'{MODULE}-{variant}', variant,
                                             output / 'lookup-contract-types' / variant)
                for variant in VARIANTS
            }
            source_guard()
        report['complete'] = True
    except Exception as exc:
        report['error'] = str(exc)
    report['seconds'] = time.monotonic() - started
    write_json(output / 'module-result.json', report)
    print(json.dumps(report, ensure_ascii=False))
    return 0 if report['complete'] else 1


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', required=True, help='reviewed integration base commit')
    purpose = parser.add_mutually_exclusive_group(required=True)
    purpose.add_argument('--plan', action='store_true')
    purpose.add_argument('--output', type=Path, help='new private evidence directory')
    parser.add_argument('--formal-lock', type=Path, help='existing operator formal.lock for execution')
    parser.add_argument('--registered-native', type=Path,
                        help='original current Stage1 native-evidence path for a mapped registered selection')
    parser.add_argument('--reuse-native', type=Path, help='original standalone Inbox Lookup native receipt')
    parser.add_argument('--reuse-native-sha256', help='published SHA256 of that original standalone receipt')
    parser.add_argument('--lookup-only', action='store_true',
                        help='check the original Lookup views while mapped downstream work remains pending')
    parser.add_argument('--prepare-native', action='store_true', help='produce the standalone native receipt only')
    args = parser.parse_args(argv)
    if args.prepare_native and (not args.output or args.registered_native or args.reuse_native or args.reuse_native_sha256):
        parser.error('--prepare-native requires --output and cannot consume native evidence')
    if args.lookup_only and not (args.reuse_native and args.reuse_native_sha256 and args.output):
        parser.error('--lookup-only requires --output and the original scoped native receipt and SHA256')
    os.umask(0o077)
    try:
        candidate = clean_candidate()
        base, paths = changed_files(args.base, candidate['commit'])
        plan = dict(select(paths), base_commit=base)
        reuse = None
        if args.reuse_native is not None or args.reuse_native_sha256 is not None:
            if not (args.reuse_native and args.reuse_native_sha256 and args.output) or args.registered_native:
                raise ValueError('standalone native reuse requires --output, --reuse-native and --reuse-native-sha256')
            local = local_lookup_impact(paths, base) if args.lookup_only else None
            permitted = sorted(paths) if local is not None else native_reuse_paths(paths, base)
            reuse = (args.reuse_native.absolute(), args.reuse_native_sha256)
            plan = dict(select([WRONG_PAGE] if local is None else [str(HERE/'module_acceptance.py')]),
                        base_commit=base, changed_paths=paths,
                        native_reuse_paths=permitted,
                        checks=['reuse-original-native', 'formal-correct', 'formal-key-only'])
            if local is not None:
                plan.update(status='lookup-only', pending_downstream=local['pending_jobs'],
                            unmapped_downstream=local['unmapped'])
        impact = registered_impact(paths, base=base) if plan['status'] == 'review-required' else None
        if impact is not None:
            plan['registered_impact'] = impact
            plan['observed_package_references'] = plan.pop('direct_consumers')
            plan['affected_modules'] = impact['affected_modules']
            plan['reason'] = ('selected registered jobs require the original current Stage1 native evidence; '
                              'the plan alone issues no module pass')
        native_only_baseline = args.prepare_native and plan['status'] == 'empty'
        if args.plan or (not native_only_baseline and plan['status'] not in ('ready', 'lookup-only')
                         and (impact is None or args.registered_native is None)):
            print(json.dumps(dict(plan, candidate=candidate), ensure_ascii=False))
            return 0 if plan['status'] == 'ready' else 2
        if not args.prepare_native and (args.formal_lock is None or args.formal_lock.is_symlink()
                                        or not args.formal_lock.is_file()):
            raise ValueError('execution requires the existing regular operator --formal-lock')
        if args.registered_native is not None:
            if impact is None:
                raise ValueError('registered evidence cannot be supplied for an unmapped or isolated selection')
            return execute_registered(args.output, plan, candidate, args.formal_lock, args.registered_native)
        # The login-session identifier is not a test input. Remove it before
        # both fingerprinting and launching the standalone children.
        os.environ.pop('XDG_SESSION_ID', None)
        if args.prepare_native:
            return execute_native_preparation(args.output, candidate)
        if args.lookup_only:
            return execute(args.output, plan, candidate, args.formal_lock, reuse, lookup_only=True)
        return execute(args.output, plan, candidate, args.formal_lock, reuse)
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        parser.exit(2, str(exc) + '\n')


if __name__ == '__main__':
    raise SystemExit(main())
