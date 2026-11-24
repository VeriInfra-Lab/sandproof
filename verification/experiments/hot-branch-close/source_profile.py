"""Exact archived candidate selection for the branch-close component."""

import fcntl
from hashlib import sha256
import json
import os
from pathlib import Path
import platform
import stat
import subprocess
from tempfile import TemporaryDirectory


def require(condition, message):
    if not condition:
        raise ValueError(message)


def git(checkout, *arguments):
    return subprocess.check_output(['git', '-C', checkout, *arguments])


def linux_merge_tree(checkout, base, head):
    # The host's Git predates `merge-tree --write-tree`. Reperform the same
    # merge in a private checkout and require the independently pinned tree.
    with TemporaryDirectory(prefix='sandproof-merge-tree-') as temporary:
        work = Path(temporary) / 'checkout'
        subprocess.run(['git', 'clone', '--quiet', '--shared', '--no-checkout',
                        '--no-tags', '--', checkout, work], check=True)
        git(work, '-c', 'advice.detachedHead=false', 'checkout', '--quiet', '--detach', base)
        git(work, '-c', 'core.hooksPath=/dev/null', '-c', 'user.name=Sandproof proof replay',
            '-c', 'user.email=proof-replay@localhost', '-c', 'rerere.enabled=false',
            '-c', 'merge.autoStash=false', 'merge', '--no-ff', '--no-commit', '--no-edit', head)
        require(not git(work, 'ls-files', '--unmerged'), 'native integration merge has unresolved files')
        return git(work, 'write-tree').decode().strip()


def linux_admission(output):
    require(platform.system() == 'Linux' and platform.machine() == 'x86_64',
            'the integration replay needs its fixed native Linux host')
    require(output.is_relative_to('/mnt/data2tb'), 'use an isolated data-disk output')
    require(os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == 'formal',
            'use the operator-owned formal worker')
    cpus = sorted(os.sched_getaffinity(0))
    require(len(cpus) == 4, 'use the four CPUs reserved by the formal worker')
    path = Path(os.environ.get('SANDPROOF_LOCK_FILE', ''))
    raw_fd = os.environ.get('SANDPROOF_LOCK_FD', '')
    require(path.is_absolute() and not path.is_symlink() and raw_fd.isascii() and raw_fd.isdecimal(),
            'the original formal lock descriptor must be inherited')
    fd = int(raw_fd)
    before, expected = os.fstat(fd), path.stat()
    require(stat.S_ISREG(before.st_mode) and stat.S_ISREG(expected.st_mode)
            and (before.st_dev, before.st_ino) == (expected.st_dev, expected.st_ino),
            'the inherited descriptor belongs to a different path')
    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    return dict(cpus=cpus, lock=dict(path=str(path), device=before.st_dev, inode=before.st_ino))


def source_profile(checkout, inventory, approved_linux, candidate=None):
    require(candidate is None or approved_linux, "candidate override requires the Linux profile")
    head = inventory['source_commit']
    exact = git(checkout, 'rev-parse', head + '^{tree}').decode().strip()
    require(exact == inventory['source_tree'], 'reviewed PR head tree differs')
    if not approved_linux:
        return head, exact, inventory['toolchain'], {}

    merge = inventory['linux_merge']
    base = merge['integration']
    observed = linux_merge_tree(checkout, base, head)
    require(observed == merge['tree'], 'integration merge tree differs')
    config = git(checkout, 'show', observed + ':verification/foundations-environment.json')
    require(sha256(config).hexdigest() == merge['environment_sha256'],
            'fixed Linux tool contract differs')
    environment = json.loads(config)
    require(environment['target'] == 'x86_64-unknown-linux-gnu'
            and 'leanprover/lean4:v' + environment['lean'] == inventory['toolchain']['lean'],
            'fixed Linux tool target or Lean release differs')
    tools = dict(rust_target=environment['target'], lean=inventory['toolchain']['lean'],
                 charon=environment['charon']['binary_sha256'],
                 aeneas=environment['aeneas']['binary_sha256'])
    identity = dict(integration_base=base, pr_head=head, merge_tree=observed)
    if candidate is None:
        return observed, observed, tools, identity
    require(candidate == merge['combined_commit'], 'unreviewed combined integration candidate')
    combined = git(checkout, 'rev-parse', candidate + '^{tree}').decode().strip()
    require(combined == merge['combined_tree'], 'reviewed combined tree differs')
    changed = git(checkout, 'diff', '--name-only', observed, combined).decode().splitlines()
    require(changed == [merge['combined_changed_path']], 'combined candidate changed the reviewed boundary')
    return candidate, combined, tools, dict(identity, combined_commit=candidate)
