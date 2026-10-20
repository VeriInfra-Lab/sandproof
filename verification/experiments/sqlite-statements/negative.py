"""Prepare/extract the actual wrong UPDATE branch; no semantic success claim.

The formal refutation must subsequently use the correct program's independent
contract and unedited wrong generated body in its own project.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

HERE=Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare_wrong(archive, destination):
    spec=importlib.util.spec_from_file_location('sqlite_statement_negative_materializer',HERE/'prepare.py')
    module=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    original=module.prepare_archive(archive,destination)
    path=destination/'controller-service/src/journal/statement_core.rs'
    before=path.read_text()
    if before.count('if changed != 1 {')!=1:
        raise ValueError('wrong branch mutation must have one exact original target')
    path.write_text(before.replace('if changed != 1 {','if changed > 1 {'))
    record=dict(kind='one-real-Rust-row-count-mutation-not-formal-counterexample',
                materialization=original,mutation_file=str(path.relative_to(destination)),
                before_sha256=hashlib.sha256(before.encode()).hexdigest(),after_sha256=sha(path),
                before='if changed != 1 {',after='if changed > 1 {')
    (destination/'artifacts/sqlite-statements-negative-source.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-archive',type=Path,required=True)
    parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--tool-root',type=Path,required=True)
    args=parser.parse_args()
    if args.output.exists() or args.output.is_symlink():
        raise ValueError('wrong extraction output already exists; inspect original task')
    print(json.dumps(prepare_wrong(args.source_archive.resolve(strict=True),args.source.absolute()),indent=2),flush=True)
    command=[sys.executable,str(HERE/'translate.py'),'--source',str(args.source.absolute()),
             '--output',str(args.output.absolute()),'--tool-root',str(args.tool_root.resolve(strict=True)),
             '--variant','accept-zero-changes']
    raise SystemExit(subprocess.run(command,check=False).returncode)
