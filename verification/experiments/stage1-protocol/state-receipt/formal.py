"""Add the decoded state/receipt proofs to the original correct caller view."""
import argparse
from copy import deepcopy
import importlib.util
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/state-receipt')
PARENT = HERE.parent/'protocol-record'
spec = importlib.util.spec_from_file_location('state_receipt_original_entry', ROOT/PARENT/'formal.py')
original = importlib.util.module_from_spec(spec)
spec.loader.exec_module(original)
parent_inventory = original.check_inventory
sha = original.sha


def check_inventory(root):
    value = deepcopy(parent_inventory(root))
    own = json.loads((root/HERE/'inventory.json').read_text())
    if sha(root/PARENT/'inventory.json') != own['parent_inventory_sha256']:
        raise ValueError('reviewed parent view changed; inspect semantics before adopting')
    for name, digest in own['shared_models'].items():
        if sha(root/name) != digest:
            raise ValueError('shared protocol/state contract changed: '+name)
    view = value['views']['correct']
    for name, item in own['files'].items():
        if sha(root/item['provider']) != item['sha256']:
            raise ValueError('state/receipt proof provider changed: '+name)
        value['providers'][name] = dict(path=item['provider'], installed_sha256=item['sha256'])
        view['audits'][name] = item['audits']
        view['required_audits'].extend(item['audits'])
    view['files'] = sorted({*view['files'], *own['files']})
    value['same_models'].update(own['representation_models'])
    return value


def run(output, require_native=False, development_native=None, prepare_only=False):
    # All extraction, copying, native admission, physical-file diagnostics,
    # dependency audits and unchanged-input checks remain in the original entry.
    original.check_inventory = check_inventory
    result = original.run(output, 'correct', require_native, development_native, prepare_only)
    (output/'parent-result.json').write_bytes((output/'result.json').read_bytes())
    result.update(kind='state-receipt-formal-development',
        parent_kind=result['kind'], parent_result_sha256=sha(output/'parent-result.json'),
        native_case='stage1-protocol-record', new_native_case=False,
        actual_async_constructor_connection_proved=False, json_bytes_proved=False,
        transaction_proved=False, public_registration_modified=False)
    (output/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=['correct'], default='correct')
    parser.add_argument('--require-native', action='store_true')
    parser.add_argument('--development-native', type=Path)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.require_native, args.development_native, args.prepare_only)
    print(json.dumps({k:result.get(k) for k in ('kind','complete','prepared','seconds','error')}))
    raise SystemExit(0 if result['complete'] or result.get('prepared') and 'error' not in result else 1)
