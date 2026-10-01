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

CHROMIUM_LIFECYCLE_PATH = '/usr/local/lib/sandproof/guest_chromium.py'

REPAIRS = {
    'filter-js-from-html': '9686e12af48346d2df538c3b46710122ee8dc80dfd0eecacb7a1476bf892ec9f',
    'git-multibranch': '300ca96b79db8d77f14742109aaf76048aafc4644ced750b1bc5c2761e8d0175',
    'gpt2-codegolf': '8c288a53bdf566d7278a35eda08d6baaf50883b0e1917ceb638b823cf6456b70',
    'build-cython-ext': '41bbdde88617070366a70e518eedca466a4ea1acd7e96899b2fb7da988e40abc',
    'mcmc-sampling-stan': 'd084666cc006911c5c80959969fba5c5699cbac92ab172db5c67eca00b1d0a97',
    'make-doom-for-mips': 'a9cd034a5e76fa2b22481717a9669869214f43fec5b4fedc957d83dd2ef6a84a',
    'build-pmars': '105292ca0e8bc399831550fc54c451002bd300644eb9beef1760dbfa7f8b7543',
    'protein-assembly': '3502b801f1260857a68a2ecbdc4497f8f7034e6a472990cd5fb9cd0eebc7dec9',
    'pytorch-model-recovery': '175a7802777b43e0fa4f629d71f7c68fb40706813c342aaddc72108d89bd5b2c',
    'qemu-alpine-ssh': '1cd31186c427b91bb00499d22aa4f0214fc1ca93b40b6e4cfa969611b848db2a',
    'qemu-startup': 'a6a644e03c4bbde4039f222d9509a884e62b4a3402990482688b67ed5dc7919d',
    'rstan-to-pystan': '069ac635c9ceda6d5235995ce7f005d924b911e3499895fb3bbad2311be56518',
    'build-pov-ray': '67c962a8b114fa1d55d1230c7d87309aa2b5ad643d9a955a2d74755fa8842e25',
    'crack-7z-hash': '2640463e6e14658426f3ea629e0702688e00553eb8006cf31ec3b9cff75c4b19',
    'caffe-cifar-10': '771887de1ed557269c7f3160c777519aa9538e1e9b59771b2f6ef38c8952490c',
    'install-windows-3.11': '68a9221b9a29331b66fd34fccd71db71d2b5a720e917f2ef95f4a8a542a71ee4',
    'torch-tensor-parallelism': 'd083b7cbf9dbb6cc2ff38c8f2acc150f0059c04aea42d6c2183aeaecc5a19385',
    'mteb-leaderboard': '4ed68831d4acf5d80d2d73eec1f21efd0896a6f51f23020f572cfc0af6da0410',
}
POV_ARCHIVES = {
    'POVDOC.TAR.Z': 'e70e44d1fe8835c4dff7c7a55bd6629b15e6a15b2ab7f2f49ee9e2dc016cc470',
    'POVSCN.TAR.Z': '4272e2d4724d8dfd916d68827194577221d17b733d99e84e7040f3a9f7eb92a7',
    'POVSRC.TAR.Z': '4d8a7073fadaca82827f1354428393cd13e4d3f71a5a3149fd7d6fffd77293d4',
}


def prepare_repaired_task(name, original, destination):
    """Give the unchanged Harbor agent a complete, independently rooted task.

    Harbor checks that its solution belongs to its task root. A replacement
    solution path under a different root is correctly rejected by that check.
    The original task remains the verifier's source of truth.
    """
    if any(path.is_symlink() for path in original.rglob('*')):
        raise InvalidInput('task symlinks are unsupported')
    shutil.copytree(original, destination,
                    ignore=lambda directory, names: ['solution'] if Path(directory) == original else [])
    repair_solution(name, original / 'solution', destination / 'solution')
    (destination / 'oracle-repair.json').rename(destination.parent / 'oracle-repair.json')
    return destination


def repair_solution(name, original, destination):
    if name not in REPAIRS:
        raise InvalidInput('no reviewed reference-solution repair for ' + name)
    files = sorted(path for path in original.rglob('*') if path.is_file())
    if any(path.is_symlink() for path in original.rglob('*')):
        raise InvalidInput('solution symlinks are unsupported')
    old = (original / 'solve.sh').read_text()
    if hashlib.sha256((original / 'solve.sh').read_bytes()).hexdigest() != REPAIRS[name]:
        raise InvalidInput('reference solution no longer matches the reviewed repair')
    if name == 'filter-js-from-html':
        # The original verifier kills ChromeDriver before quit, leaving live
        # browser trees behind. The explicit repaired run installs the existing
        # driver-lifetime supervisor; run_task selects it only for the verifier.
        # Preserve the complete filter implementation and all browser options.
        new = ('install -D -m 755 /solution/guest_chromium.py '
               + CHROMIUM_LIFECYCLE_PATH + ' || exit $?\n' + old)
    elif name == 'git-multibranch':
        # The original image omits tmux, but the solution's final command
        # requires it. Install the missing tool before configuring services;
        # retain the original tmux command and all Git/SSH/nginx behavior.
        marker = 'set -e\n'
        if old.count(marker) != 1:
            raise InvalidInput('reference solution no longer matches the reviewed repair')
        new = old.replace(marker, marker + '''
if ! command -v tmux >/dev/null 2>&1; then
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends tmux
fi
''')
    elif name == 'gpt2-codegolf':
        # The original arena reserves 6.75 GiB for the 124M model, while pages
        # are touched only as the unchanged model uses them. Keep the same
        # address range with Linux demand paging and check both allocations.
        # Guest kernel policy, physical memory and address-space limits apply.
        include = '#include<stdio.h>\n'
        allocation = 'D=malloc(2LL*V*V*C*zz);\nbpe=malloc(1e9);'
        if old.count(include) != 1 or old.count(allocation) != 1:
            raise InvalidInput('reference solution no longer matches the reviewed repair')
        new = old.replace(include, include + '#include<sys/mman.h>\n')
        new = new.replace(allocation, '''D=mmap(0,2LL*V*V*C*zz,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS|MAP_NORESERVE,-1,0);
if(D==MAP_FAILED)return 77;
bpe=malloc(1e9);
if(!bpe)return 78;''')
    elif name == 'build-cython-ext':
        # pyknotid 0.5.3 consumes planarity's pre-1.0 node attribute names.
        # Pin the compatible dependency in the same editable installation;
        # keep the task's NumPy, Cython extensions and repository tests intact.
        marker = 'pip install -e .'
        if old.count(marker) != 1:
            raise InvalidInput('reference solution no longer matches the reviewed repair')
        new = old.replace(marker, 'pip install "planarity==0.6" -e .')
    elif name == 'mcmc-sampling-stan':
        # RcppParallel's current source requires cmake. Keep the original R,
        # RStan pins, model, sampling procedure and budget. Non-interactive
        # remotes otherwise upgrades already installed dependencies, replacing
        # the pinned StanHeaders compiler with one rejecting the old model.
        marker = 'sudo apt-get install -y \\\n'
        if old.count(marker) != 1:
            raise InvalidInput('reference solution no longer matches the reviewed repair')
        new = old.replace(marker, 'export R_REMOTES_UPGRADE=never\n' + marker + '    cmake \\\n')
    elif name == 'make-doom-for-mips':
        # The OCI image's apt indices reference packages removed by updates.
        # Refresh them in the guest, inside the unchanged oracle deadline.
        marker = 'apt install -y clang gcc-mips-linux-gnu g++-mips-linux-gnu llvm llvm-dev lld'
        if old.count(marker) != 1:
            raise InvalidInput('reference solution no longer matches the reviewed repair')
        new = old.replace(marker, 'apt-get update && ' + marker + ' || exit $?')
    elif name == 'build-pmars':
        # Keep the package set and original Debian source/build commands;
        # use the refreshed configured repository's versions. The run records
        # the actual installed package inventory, not the obsolete pins.
        new = old
        for package in ('build-essential=12.12', 'libncurses-dev=6.5+20250216-2',
                        'ca-certificates=20250419', 'dpkg-dev=1.22.21', 'devscripts=2.25.15+deb13u1'):
            if new.count(package) != 1:
                raise InvalidInput('reference solution no longer matches the reviewed repair')
            new = new.replace(package, package.split('=')[0])
    elif name == 'protein-assembly':
        start = old.index('def get_single_fasta(pdb_id):')
        end = old.index('def get_pdb_title(pdb_id):', start)
        new = old[:start] + 'from protein_sequences import get_single_fasta\n\n\n' + old[end:]
        marker = 'python pdb_ids.py'
        if new.count(marker) != 1:
            raise InvalidInput('reference solution no longer matches the reviewed repair')
        new = new.replace(marker, 'cp /solution/protein_sequences.py /app/protein_sequences.py\n' + marker)
    elif name == 'pytorch-model-recovery':
        new = 'python /solution/guest_wheel_sharing.py || exit $?\n' + old
    elif name in ('qemu-alpine-ssh', 'qemu-startup'):
        new = 'bash /solution/guest_bullseye_snapshot.sh || exit $?\n' + old
    elif name == 'rstan-to-pystan':
        new = ('bash /solution/guest_pystan_dependencies.sh || exit $?\n'
               'export PIP_CONSTRAINT=/solution/pystan-constraints.txt\n' + old)
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
    elif name == 'mteb-leaderboard':
        # The solution immediately checks out its pinned historical commit.
        # Preserve the complete clone, but do not first materialize today's
        # larger HEAD tree, which can fill the original 10 GiB task disk.
        marker = 'git clone https://github.com/embeddings-benchmark/results'
        if old.count(marker) != 1:
            raise InvalidInput('reference solution no longer matches the reviewed repair')
        new = old.replace(marker, 'git clone --no-checkout https://github.com/embeddings-benchmark/results')
        # In MTEB 1.38.41 results_repo is still a clone source, even for a
        # local path. Its documented cache root must point at the parent of
        # this complete checkout to avoid a second full working tree.
        marker = 'python leaderboard.py'
        if new.count(marker) != 1:
            raise InvalidInput('reference solution no longer matches the reviewed repair')
        new = new.replace(marker, 'MTEB_CACHE="$(dirname "$PWD")" python leaderboard.py')
    elif name == 'torch-tensor-parallelism':
        # In Torch 2.7.0, the direct Gloo Python holder releases the GIL on
        # destruction; its parent ProcessGroup's holder does not. Keep the
        # backend owned by the layer so its worker-thread join uses that holder.
        # No collective, model computation, test or package is replaced.
        marker = '        self.rank = dist.get_rank()\n'
        if old.count(marker) != 2:
            raise InvalidInput('reference solution no longer matches the reviewed repair')
        new = old.replace(marker, marker + '''
        # Keep Gloo alive through ProcessGroup teardown. The backend's own
        # Python holder releases the GIL while joining its worker threads.
        self._gloo_backend = (
            dist.distributed_c10d._get_default_group()._get_backend(torch.device("cpu"))
            if dist.get_backend() == "gloo" else None
        )
''')
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
    if name == 'pytorch-model-recovery':
        shutil.copyfile(Path(__file__).with_name('guest_wheel_sharing.py'), destination / 'guest_wheel_sharing.py')
    if name in ('qemu-alpine-ssh', 'qemu-startup'):
        shutil.copyfile(Path(__file__).with_name('guest_bullseye_snapshot.sh'), destination / 'guest_bullseye_snapshot.sh')
    if name == 'rstan-to-pystan':
        for filename in ('guest_pystan_dependencies.sh', 'pystan-constraints.txt'):
            shutil.copyfile(Path(__file__).with_name(filename), destination / filename)
    if name == 'filter-js-from-html':
        shutil.copyfile(Path(__file__).with_name('guest_chromium.py'), destination / 'guest_chromium.py')
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
