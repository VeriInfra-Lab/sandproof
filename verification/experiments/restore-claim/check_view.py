"""Check the private extraction's complete local import closure and service audits."""
import argparse
import json
import os
from pathlib import Path
import subprocess


def check(project, output, targets):
    protocol = project / 'verification/experiments/stage1-protocol/formal.py'
    from runpy import run_path
    formal = run_path(str(protocol))
    resolver = formal['load_module']('restore_claim_closure', project / 'tools/proof_batch.py')
    modules = 'SandboxModel/Lifecycle/Protocol/RestoreClaim'
    seeds = [f'{modules}/{target}.lean' for target in targets]
    files = sorted(resolver.closure(project, seeds, active_view='lakefile.toml'))

    def run(source, logs, label, command):
        with (logs / (label + '.log')).open('xb') as log:
            subprocess.run(command, cwd=source, stdin=subprocess.DEVNULL, stdout=log,
                           stderr=subprocess.STDOUT, check=True, timeout=240)

    output.mkdir()
    result = formal['actual_files'](project, files, output, dict(os.environ), run)
    (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('project', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('targets', nargs='+')
    args = parser.parse_args()
    check(args.project.resolve(), args.output.resolve(), args.targets)
