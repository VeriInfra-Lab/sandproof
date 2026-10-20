"""Materialize this lane's frozen actual-call candidate without production edits.

The source is a complete Git archive of the named production commit. Only the
registered original journal file and new actually called core may differ.
This prepares source; it does not report a successful extraction or proof.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

HERE=Path(__file__).resolve().parent
SOURCE_FILES={'controller-service/src/journal.rs','controller-service/src/lib.rs','Cargo.lock','controller-service/Cargo.toml'}
CANDIDATE_FILES={'controller-service/src/journal.rs','controller-service/src/journal/statement_core.rs'}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _profile():
    profile=json.loads((HERE/'candidate.json').read_text())
    if set(profile['source_files'])!=SOURCE_FILES or set(profile['candidate_files'])!=CANDIDATE_FILES:
        raise ValueError('profile cannot change the fixed SQL candidate file scope')
    if profile['patch']!='candidate-core.patch':
        raise ValueError('patch must be the registered local candidate-core.patch')
    patch=HERE/profile['patch']
    if sha(patch)!=profile['patch_sha256']:
        raise ValueError('candidate patch changed')
    return profile


def _destination(destination):
    destination=destination.absolute()
    if destination.exists() or destination.is_symlink():
        raise ValueError('destination already exists: '+str(destination))
    return destination


def prepare(source_base, destination):
    source_base=source_base.resolve(strict=True)
    destination=_destination(destination)
    profile=_profile()
    base=profile['base']
    actual=subprocess.check_output(['git','rev-parse',base+'^{commit}'],cwd=source_base,text=True).strip()
    if actual!=base:
        raise ValueError('production base is not the exact registered commit')
    for name, expected in profile['source_files'].items():
        raw=subprocess.check_output(['git','show',base+':'+name],cwd=source_base)
        if hashlib.sha256(raw).hexdigest()!=expected:
            raise ValueError('frozen production source changed: '+name)
    raw=subprocess.check_output(['git','archive','--format=tar',base],cwd=source_base)
    return _materialize(raw,destination,profile)


def prepare_archive(archive, destination):
    destination=_destination(destination)
    profile=_profile()
    raw=archive.read_bytes()
    if hashlib.sha256(raw).hexdigest()!=profile['source_archive_sha256']:
        raise ValueError('frozen source archive changed')
    return _materialize(raw,destination,profile)


def _materialize(raw, destination, profile):
    patch=HERE/profile['patch']
    base=profile['base']
    destination.mkdir(parents=True,mode=0o700)
    with tarfile.open(fileobj=io.BytesIO(raw)) as tree:
        tree.extractall(destination,filter='data')
    originals={str(p.relative_to(destination)):sha(p) for p in destination.rglob('*')
               if p.is_file() and not p.is_symlink()}
    for name, expected in profile['source_files'].items():
        if sha(destination/name)!=expected:
            raise ValueError('archive source mismatch: '+name)
    subprocess.run(['git','apply','--check',str(patch)],cwd=destination,check=True)
    subprocess.run(['git','apply',str(patch)],cwd=destination,check=True)
    after={str(p.relative_to(destination)):sha(p) for p in destination.rglob('*')
           if p.is_file() and not p.is_symlink()}
    changed={name for name in originals.keys()|after.keys() if originals.get(name)!=after.get(name)}
    if changed!=set(profile['candidate_files']):
        raise ValueError('candidate changed files outside registered scope: '+str(sorted(changed)))
    for name, expected in profile['candidate_files'].items():
        if after.get(name)!=expected:
            raise ValueError('candidate source mismatch: '+name)
    record=dict(base=base,source_archive_sha256=hashlib.sha256(raw).hexdigest(),
                patch_sha256=profile['patch_sha256'],candidate_files=profile['candidate_files'],
                changed=sorted(changed),scope='source materialization only, not a build/test/proof')
    evidence=destination/'artifacts/sqlite-statements-materialization.json'
    evidence.parent.mkdir(exist_ok=True)
    evidence.write_text(json.dumps(record,indent=2)+'\n')
    return record


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    sources=parser.add_mutually_exclusive_group(required=True)
    sources.add_argument('--source-base',type=Path)
    sources.add_argument('--source-archive',type=Path)
    parser.add_argument('--destination',type=Path,required=True)
    args=parser.parse_args()
    record=(prepare(args.source_base,args.destination) if args.source_base else
            prepare_archive(args.source_archive,args.destination))
    print(json.dumps(record,indent=2))
