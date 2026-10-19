"""Check selected actual Rust bodies survived the isolated guest translation.

This checks an extraction inventory, not Lean compilation or proof closure.
"""
from __future__ import annotations

import json
from pathlib import Path
import sys


def check_inventory(roots: list[str], inventory: dict) -> list[str]:
    if not roots or len(set(roots)) != len(roots):
        raise ValueError('extraction roots must be nonempty and distinct')
    if inventory['crate'] != 'sandproof_guest_core':
        raise ValueError('wrong translated crate')
    functions = inventory['functions']
    for root in roots:
        prefix, owner, method = root.rsplit('::', 2)
        if prefix not in ('crate::command', 'crate::transfer'):
            raise ValueError(f'unexpected guest extraction root: {root}')
        module = prefix.replace('crate::', 'sandproof_guest_core::', 1)
        rust_name = f'{module}::{{{module}::{owner}}}::{method}'
        matches = [f for f in functions if f['rust_name'] == rust_name]
        if len(matches) != 1 or matches[0]['is_local'] is not True or matches[0]['is_opaque'] is not False:
            raise ValueError(f'missing or opaque actual Rust body: {root}')
    if any(f['is_local'] and f['is_opaque'] for f in functions):
        raise ValueError('opaque local dependency is not an extracted body')
    return sorted(f['rust_name'] for f in functions if f['is_opaque'])


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit('usage: check_guest_extraction.py ROOTS.txt translation.json')
    roots = Path(sys.argv[1]).read_text().splitlines()
    inventory = json.loads(Path(sys.argv[2]).read_text())
    external = check_inventory(roots, inventory)
    print(f'Guest extraction inventory: {len(roots)} selected Rust bodies present; {len(external)} opaque external dependencies.')
    for name in external:
        print(f'External definition/contract still required: {name}')
    print('Inventory check only; not Lean compilation or proof acceptance.')


if __name__ == '__main__':
    main()
