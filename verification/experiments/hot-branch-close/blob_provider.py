"""Select the original pinned two-BLOB provider files from the same source archive."""

import json
from pathlib import Path
import tarfile


BLOB = Path('verification/experiments/stage1-protocol/standard-save-digest/blob')
INSTALLER = BLOB / 'materialize.py'
INVENTORY = BLOB / 'inventory.json'
COMBINED_COMMON_INPUTS = (
    'verification/aeneas/sqlite-statements/SandproofSqliteStatements/FunsExternal.lean',
    'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean',
    'SandboxModel/Foundations/ErrorMessages.lean',
)


def stage(archive_path, inventory_path, destination):
    inventory = json.loads(inventory_path.read_bytes())
    names = {str(INSTALLER), str(INVENTORY), *COMBINED_COMMON_INPUTS}
    for group in ('sources', 'providers', 'templates', 'generated'):
        names.update(inventory[group])
    destination.mkdir(mode=0o700)
    with tarfile.open(archive_path) as archive:
        members = [archive.getmember(name) for name in sorted(names)]
        if not all(member.isfile() for member in members):
            raise ValueError('original BLOB materialization inputs must be regular Git files')
        archive.extractall(destination, members=members, filter='data')
    return destination / INSTALLER
