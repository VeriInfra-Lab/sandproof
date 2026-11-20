"""Manifest mutations through the existing native baseline/wrong/restored checks.

Run on the ordinary test machine. This does not produce a shared native seal.
"""
from pathlib import Path
import argparse
import importlib.util
import json
import os

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent


def run(output, target, archive):
    path = ROOT / 'verification/experiments/stage1-protocol/negative_native.py'
    spec = importlib.util.spec_from_file_location('manifest_original_native', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.MUTATIONS = json.loads((HERE / 'mutations.json').read_text())
    return module.run(output, target, archive)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-archive', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--target', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.resolve(), args.target.resolve(), args.source_archive.resolve())
    print(json.dumps({k: result.get(k) for k in ['kind', 'success', 'seconds', 'error']}))
    raise SystemExit(0 if result['success'] else 1)
