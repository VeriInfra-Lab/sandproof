"""Keep the original tool installation visible when rebinding a private source."""
from pathlib import Path
import os

VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')


def tool_root(root):
    original = root / '.tools'
    selected = Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT', str(original))).resolve(strict=True)
    if selected == VIRTUAL / '.tools':
        raise ValueError('a stable tool alias outside the rebound source is required')
    if not selected.is_dir() or not selected.samefile(original):
        raise ValueError('stable tool alias must identify the original fixed installation')
    return selected


def tool_bind(root):
    selected = str(tool_root(root))
    return ['--ro-bind', selected, selected]
