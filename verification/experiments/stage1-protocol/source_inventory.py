"""Admit the exact unified source module set, while retaining each case's roots."""
from pathlib import Path
import json

HERE = Path('verification/experiments/stage1-protocol')


def retained(project, required):
    data = json.loads((project / HERE / 'retained-protocol.json').read_text())
    names = data['modules']
    paths = list((project / 'SandboxModel/Lifecycle/Protocol').glob('*.lean'))
    if any(p.is_symlink() or not p.is_file() for p in paths):
        raise ValueError('unregistered or missing retained protocol proof file')
    actual = {p.stem for p in paths}
    if (data.get('schema') != 1 or len(names) != len(set(names))
            or actual != set(names) or not set(required).issubset(actual)):
        raise ValueError('unregistered or missing retained protocol proof file')
    return actual
