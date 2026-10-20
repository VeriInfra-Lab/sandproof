"""Admit the exact reviewed input and literal reification; never refresh it."""
import json
from pathlib import Path

from source import ROOT, lean_source, source_record


def check(root=ROOT):
    frozen = json.loads((root / 'verification/experiments/redis-lua/source-map.json').read_text())
    if frozen.get('schema') != 1 or frozen.get('scope') != 'source-bound normal control paths':
        raise ValueError('unreviewed Lua batch scope')
    record, scripts = source_record(root)
    if record != frozen['source']:
        raise ValueError('original Lua or Rust caller changed; a new reviewed batch is required')
    expected = lean_source(scripts)
    actual = (root / 'SandboxModel/CenterStore/Lua/RawSource.lean').read_text()
    if actual != expected:
        raise ValueError('Lean raw input differs from the actual frozen literal bytes')
    print(json.dumps({'source_admitted': True, 'scripts': record['scripts'],
                      'scope': frozen['scope'], 'complete_lua_or_store_proof': False}))


if __name__ == '__main__':
    check()
