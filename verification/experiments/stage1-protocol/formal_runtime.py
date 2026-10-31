"""Keep the original tool installation visible when rebinding a private source."""
from pathlib import Path
import os

VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
PACKAGE_ALIAS = Path('/home/ning/sandproof-proof-v0.yml6x1jB/src/.lake/packages')


def tool_root(root):
    original = root / '.tools'
    selected = Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT', str(original))).resolve(strict=True)
    if selected == VIRTUAL / '.tools':
        raise ValueError('a stable tool alias outside the rebound source is required')
    if not selected.is_dir() or not selected.samefile(original):
        raise ValueError('stable tool alias must identify the original fixed installation')
    return selected


def package_root(root, requested=None):
    original = root / '.lake/packages'
    selected = Path(os.environ.get('SANDPROOF_FORMAL_PACKAGES',
        str(PACKAGE_ALIAS if root == VIRTUAL else original))).resolve(strict=True)
    if selected.is_relative_to(VIRTUAL):
        raise ValueError('a stable package alias outside the rebound source is required')
    if not selected.is_dir() or not selected.samefile(original):
        raise ValueError('stable package alias must identify the original fixed packages')
    if requested is not None and not selected.samefile(requested):
        raise ValueError('requested packages differ from the original fixed packages')
    return selected


def tool_bind(root):
    # These are the original operator's read-only seed mounts. Re-expose only
    # these runtime directories after the inner namespace clears /home.
    runtime = [Path(os.environ[name]).resolve(strict=True)
               for name in ('CARGO_HOME', 'RUSTUP_HOME', 'ELAN_HOME')]
    runtime.append(Path(os.environ.get('SANDPROOF_PROOF_RUNTIME_ROOT',
        '/home/ning/.local/share/sandproof-proof-runtime')).resolve(strict=True))
    if any(not path.is_dir() for path in runtime):
        raise ValueError('original fixed runtime directory is missing')
    directories = dict.fromkeys([tool_root(root), package_root(root), *runtime])
    return [part for path in directories for part in ('--ro-bind', str(path), str(path))]
