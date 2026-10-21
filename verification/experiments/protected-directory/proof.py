"""Component replay: actual stock extraction, all physical Lean files and exact audits."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import traceback

from check import ROOT, HERE, MODEL, LIBRARY, NAMESPACE, CORRECT, NEGATIVE, check_inputs, sha

sys.path.insert(0, str(ROOT))
from tools.check_first_chain_extraction import check_tools


def load(name, relative):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--tool-root', type=Path, required=True)
    parser.add_argument('--std-evidence', type=Path, required=True,
                        help='complete files under version/relative-std-path; source review, not a std proof')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'linux' or not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('registered formal Linux runtime and private target required')
    check_inputs()
    tool = args.tool_root.resolve(strict=True)
    check_tools(ROOT, tool)
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False, mode=0o700)
    start = time.monotonic()
    record = {'kind': 'protected-directory-component-not-full-suite', 'files': []}
    env = dict(os.environ, LEAN_NUM_THREADS='2', LEAN_STACK_SIZE_KB='65536', PYTHONDONTWRITEBYTECODE='1')

    def run(command, name, cwd=ROOT):
        log = out / (name + '.log')
        began = time.monotonic()
        with log.open('xb') as stream:
            code = subprocess.run(command, cwd=cwd, env=env, stdout=stream,
                                  stderr=subprocess.STDOUT, timeout=1200).returncode
        row = {'command': command, 'exit_code': code, 'seconds': time.monotonic() - began,
               'log_sha256': sha(log)}
        record.setdefault('commands', []).append(row)
        if code:
            raise ValueError('component command failed: ' + name)

    try:
        sources = json.loads((HERE / 'std-sources.json').read_text())
        for version, files in sources.items():
            for name, expected in files.items():
                if sha(args.std_evidence.resolve() / version / name) != expected:
                    raise ValueError('fixed std source changed: ' + version + '/' + name)
        record['std_source_hashes'] = sources
        run([sys.executable, str(HERE / 'extract.py'), '--archive', str(args.archive.resolve()),
             '--tool-root', str(tool), '--output', str(out / 'extraction')], 'extraction')
        view = out / 'proof-view'
        view.mkdir(mode=0o700)
        paths = list((ROOT / MODEL).glob('*.lean')) + list((ROOT / LIBRARY).rglob('*.lean'))
        paths += [ROOT / n for n in ('SandboxModel/ProofAuditCommand.lean', 'lean-toolchain',
            'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh')]
        for path in paths:
            target = view / path.relative_to(ROOT)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
        shutil.copyfile(HERE / 'lakefile.toml', view / 'lakefile.toml')
        (view / '.tools').symlink_to(tool, target_is_directory=True)
        (view / '.lake').mkdir()
        packages = (ROOT / '.lake/packages').resolve(strict=True)
        (view / '.lake/packages').symlink_to(packages, target_is_directory=True)
        run(['lake', 'build', 'SandboxModel.Boundaries.ProtectedDirectory.Audit',
             'SandboxModel.Boundaries.ProtectedDirectory.BadAudit',
             'SandboxModel.Boundaries.ProtectedDirectory.ElaborationScope'], 'build', view)
        protocol = load('protected_directory_service_protocol',
                        'verification/experiments/schema-domains/replay.py')
        messages = load('protected_directory_audit_messages',
                        'verification/experiments/bounded-writer/audit_messages.py')
        actual = sorted(str(p.relative_to(view)) for p in (view / MODEL).glob('*.lean'))
        actual += sorted(str(p.relative_to(view)) for p in (view / LIBRARY).rglob('*.lean'))
        observed = []
        for i, name in enumerate(actual):
            result = protocol.diagnostic(view, name, out / ('file-' + str(i)), env)
            observed += messages.read_messages(view / name, out / ('file-' + str(i) + '.log'), result)
            record['files'].append(result)
            (out / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
        expected = [NAMESPACE + name for name in CORRECT + NEGATIVE]
        if sorted(messages.PATTERN.fullmatch(m)[1] for m in observed) != sorted(expected):
            raise ValueError('actual sixteen audit messages missing, duplicated or extra')
        check_inputs()
        if any(sha(ROOT / item['file']) != item['source_sha256'] for item in record['files']):
            raise ValueError('actual source changed during component replay')
        # Forward only the original messages admitted against all actual files.
        # The shared batch must also see the separately checked wrong version.
        for message in observed:
            print(message, flush=True)
        record.update(success=True, audits=len(expected))
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic() - start
    (out / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps({'success': record['success'], 'seconds': record['seconds'],
                      'files': len(record['files']), 'audits': record.get('audits')}))
    raise SystemExit(0 if record['success'] else 1)


if __name__ == '__main__':
    main()
