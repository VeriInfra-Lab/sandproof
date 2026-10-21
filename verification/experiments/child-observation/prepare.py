"""Materialize the one registered candidate from a frozen full Git archive."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

HERE=Path(__file__).resolve().parent
FILES={'sandbox-runtime/src/child_process.rs',
       'sandbox-runtime/src/child_process/observation_core.rs',
       'sandbox-runtime/src/child_process/observation_io.rs'}

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def prepare(archive, destination, variant='correct'):
    if variant not in ('original','correct','kill-success-release'):
        raise ValueError('unregistered candidate variant')
    profile=json.loads((HERE/'candidate.json').read_text())
    if set(profile['candidate_files'])!=FILES or profile['patch']!='candidate-core.patch':
        raise ValueError('unregistered file scope')
    patch=HERE/profile['patch']
    if sha(patch)!=profile['patch_sha256'] or sha(archive)!=profile['source_archive_sha256']:
        raise ValueError('frozen archive/patch differs')
    if destination.exists() or destination.is_symlink():
        raise ValueError('destination already exists')
    destination.mkdir(parents=True,mode=0o700)
    # This is our exact hashed Git archive, not an untrusted uploaded tarball.
    subprocess.run(['tar','-xf',str(archive),'-C',str(destination)],check=True)
    original = {str(p.relative_to(destination)): sha(p) for p in destination.rglob('*') if p.is_file()}
    for name,digest in profile['source_files'].items():
        if sha(destination/name)!=digest: raise ValueError('base source changed: '+name)
    if variant!='original':
        subprocess.run(['git','apply','--check',str(patch)],cwd=destination,check=True)
        subprocess.run(['git','apply',str(patch)],cwd=destination,check=True)
        for name,digest in profile['candidate_files'].items():
            if sha(destination/name)!=digest: raise ValueError('candidate source changed: '+name)
    changed = {str(p.relative_to(destination)) for p in destination.rglob('*') if p.is_file() and original.get(str(p.relative_to(destination))) != sha(p)}
    deleted = set(original) - {str(p.relative_to(destination)) for p in destination.rglob('*') if p.is_file()}
    expected = set() if variant == 'original' else FILES
    if changed != expected or deleted:
        raise ValueError('patch escaped registered three-file scope')
    if variant=='kill-success-release':
        p=destination/'sandbox-runtime/src/child_process/observation_core.rs'
        text=p.read_text(); before='    let _ = io.kill(child);'
        if text.count(before)!=1: raise ValueError('ambiguous mutation')
        text=text.replace(before,'''    if let Ok(()) = io.kill(child) {
        drop(owned_child.take());
        drop(owned_slot.take());
        return Err(io.unknown_exit());
    }''')
        p.write_text(text)
    return {'base':profile['base'],'variant':variant,'archive_sha256':sha(archive),
        'patch_sha256':sha(patch),'candidate_files':{p:sha(destination/p) for p in FILES if (destination/p).exists()}}

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive',type=Path,required=True)
    parser.add_argument('--destination',type=Path,required=True)
    parser.add_argument('--variant',choices=['original','correct','kill-success-release'],default='correct')
    args=parser.parse_args()
    print(json.dumps(prepare(args.archive.resolve(strict=True),args.destination.absolute(),args.variant),indent=2))
