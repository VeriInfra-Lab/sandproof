"""Opt-in reference-solution repairs, separate from original-oracle results.

Only copied solution files can change. Original inputs and scoring files remain
immutable; the selected repair, exact diff and hashes accompany every run.
"""
import difflib
import hashlib
import json
from pathlib import Path
import shutil

from tools.benchmark_tasks import InvalidInput

REPAIRS = {
    'protein-assembly': '3502b801f1260857a68a2ecbdc4497f8f7034e6a472990cd5fb9cd0eebc7dec9',
    'build-pov-ray': '67c962a8b114fa1d55d1230c7d87309aa2b5ad643d9a955a2d74755fa8842e25',
    'crack-7z-hash': '2640463e6e14658426f3ea629e0702688e00553eb8006cf31ec3b9cff75c4b19',
    'caffe-cifar-10': '771887de1ed557269c7f3160c777519aa9538e1e9b59771b2f6ef38c8952490c',
    'install-windows-3.11': '68a9221b9a29331b66fd34fccd71db71d2b5a720e917f2ef95f4a8a542a71ee4',
}
POV_ARCHIVES = {
    'POVDOC.TAR.Z': 'e70e44d1fe8835c4dff7c7a55bd6629b15e6a15b2ab7f2f49ee9e2dc016cc470',
    'POVSCN.TAR.Z': '4272e2d4724d8dfd916d68827194577221d17b733d99e84e7040f3a9f7eb92a7',
    'POVSRC.TAR.Z': '4d8a7073fadaca82827f1354428393cd13e4d3f71a5a3149fd7d6fffd77293d4',
}


def repair_solution(name, original, destination):
    if name not in REPAIRS:
        raise InvalidInput('no reviewed reference-solution repair for ' + name)
    files = sorted(path for path in original.rglob('*') if path.is_file())
    if any(path.is_symlink() for path in original.rglob('*')):
        raise InvalidInput('solution symlinks are unsupported')
    old = (original / 'solve.sh').read_text()
    if hashlib.sha256((original / 'solve.sh').read_bytes()).hexdigest() != REPAIRS[name]:
        raise InvalidInput('reference solution no longer matches the reviewed repair')
    if name == 'protein-assembly':
        start = old.index('def get_single_fasta(pdb_id):')
        end = old.index('def get_pdb_title(pdb_id):', start)
        new = old[:start] + 'from protein_sequences import get_single_fasta\n\n\n' + old[end:]
        marker = 'python pdb_ids.py'
        if new.count(marker) != 1:
            raise InvalidInput('reference solution no longer matches the reviewed repair')
        new = new.replace(marker, 'cp /solution/protein_sequences.py /app/protein_sequences.py\n' + marker)
    elif name == 'build-pov-ray':
        # Exact archive digests independently checked against upstream PR #77.
        new = old
        for archive, digest in POV_ARCHIVES.items():
            command = 'wget https://www.povray.org/ftp/pub/povray/Old-Versions/Official-2.2/' + archive
            replacement = ('wget --timeout=30 --tries=3 -O ' + archive
                           + ' https://ftp.icm.edu.pl/packages/povray/Old-Versions/Official-2.2/' + archive
                           + "\nprintf '%s\\n' '" + digest + '  ' + archive + "' | sha256sum -c")
            new = new.replace(command, replacement)
    elif name == 'crack-7z-hash':
        # Try the unmodified bundled common-password list first. No expected
        # password, special mask or task-specific candidate list is supplied.
        new = old.replace('/app/john/run/john /app/secrets.hash > /app/cracked.txt',
                          '/app/john/run/john --wordlist=/app/john/run/password.lst --rules '
                          '/app/secrets.hash > /app/cracked.txt\n'
                          '/app/john/run/john --show /app/secrets.hash > /app/recovered.txt\n'
                          "grep -q '1 password hash cracked, 0 left' /app/recovered.txt || exit 1\n"
                          "archive_password=$(head -n 1 /app/recovered.txt | cut -d: -f2)\n"
                          'test -n "$archive_password" || exit 1')
        # Remove the original answer literal: extraction must use John's result.
        new = new.replace('7z x -p1998 /app/secrets.7z -o/app',
                          '7z x "-p$archive_password" /app/secrets.7z -o/app')
    elif name == 'caffe-cifar-10':
        # Same official binary dataset, verified against the author's MD5;
        # download and extraction still consume the original agent budget.
        new = old.replace('./data/cifar10/get_cifar10.sh', '''(
    set -e
    cd data/cifar10
    wget --timeout=30 --tries=3 -O cifar-10-binary.tar.gz https://data.brainchip.com/dataset-mirror/cifar10/cifar-10-binary.tar.gz
    printf '%s\\n' 'c4a38c50a1bc5f3a1c5537f2155ab9d68f9f25eb1ed8d9ddda3db29a59bca1dd  cifar-10-binary.tar.gz' | sha256sum -c
    tar -xf cifar-10-binary.tar.gz
    rm cifar-10-binary.tar.gz
    mv cifar-10-batches-bin/* .
    rmdir cifar-10-batches-bin
) || exit 1''')
    elif name == 'install-windows-3.11':
        # The original image CMD already runs this exact bridge via supervisor.
        # Reuse only that known command, not an arbitrary listener on port 8080.
        new = old.replace('    websockify --web /usr/share/novnc 8080 localhost:5901 --daemon',
                          '''    if pgrep -f '[/]websockify --web /usr/share/novnc 8080 (127[.]0[.]0[.]1|localhost):5901' > /dev/null; then
        echo "Reusing the supervisor's existing VNC WebSocket bridge"
    else
        websockify --web /usr/share/novnc 8080 localhost:5901 --daemon
    fi''')
    shutil.copytree(original, destination)
    (destination / 'solve.sh').write_text(new)
    if name == 'protein-assembly':
        shutil.copyfile(Path(__file__).with_name('protein_sequences.py'), destination / 'protein_sequences.py')
    def digest(path):
        return hashlib.sha256(path.read_bytes()).hexdigest()
    evidence = {
        'oracle_mode': 'repaired', 'task': name,
        'scope': 'modified reference solution; original verifier and limits; NOT original-oracle acceptance',
        'original_files': {p.relative_to(original).as_posix(): digest(p) for p in files},
        'repaired_files': {p.relative_to(destination).as_posix(): digest(p)
                           for p in sorted(destination.rglob('*')) if p.is_file()},
        'diff': ''.join(difflib.unified_diff(old.splitlines(True), new.splitlines(True),
                                           fromfile='original/solve.sh', tofile='repaired/solve.sh')),
    }
    (destination.parent / 'oracle-repair.json').write_text(json.dumps(evidence, indent=2) + '\n')
    return destination, destination / 'solve.sh'
