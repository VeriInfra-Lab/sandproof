"""Check both real parent dispatches when producing and consuming the coordinator evidence."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

# The public registrar loads a component by path. Its sibling modules are the same
# ones used when the coordinator driver runs as a script.
package = str(Path(__file__).resolve().parent)
if package not in sys.path:
    sys.path.insert(0, package)

from guard_binding import check_guard
from inline_binding import inspect_inline
from sender_binding import inspect_senders
from sender_binding_regressions import check_guard_rejections, check_inline_rejections, check_rejections
from wiring import inspect_parent, require

BASE = 'd642a5f45253a716dbb8411a9942b8b9581dd5fb'
CALLER = Path('controller-service/src/application/coordinator/lifecycle_intent.rs')
AREA = Path('verification/experiments/stage1-protocol')


def parent_dispatch(view, *, refactored):
    guard = check_guard(view)
    broadcast = inspect_parent(view.raw, refactored=refactored, guard=guard)
    if not refactored:
        broadcast = inspect_inline(view, guard)
    senders = inspect_senders(view, broadcast)
    result = {'guard_local': guard, 'senders': senders}
    if refactored:
        senders['rejected_metadata'] = check_rejections(view, broadcast)
        result['rejected_guard_metadata'] = check_guard_rejections(view)
    else:
        result['rejected_inline_metadata'] = check_inline_rejections(view)
    return result


def check_parent_evidence(root, folder, result, evidence):
    """Recheck both saved Charon callers against the same dispatch checks used at capture."""
    current, tree = subprocess.check_output(['git', 'rev-parse', 'HEAD', 'HEAD^{tree}'],
                                            cwd=root, text=True).splitlines()
    require(result.get('original') == BASE and result.get('candidate') == current
            and result.get('tree') == tree,
            'coordinator checkpoint source differs')
    from tools.proof_native_suite import load
    original = load('coordinator_saved_binding', root/AREA/'serde-binding/binding.py').Original
    for side, commit, refactored in [('original', BASE, False), ('candidate', current, True)]:
        row = result.get(side+'_parent')
        path = folder/(side+'-parent')/'actual.llbc'
        require(isinstance(row, dict) and path.is_file() and not path.is_symlink(),
                'missing coordinator parent capture: '+side)
        data = path.read_bytes()
        require(row.get('ir') == evidence.get(str(path)) == hashlib.sha256(data).hexdigest(),
                'changed coordinator parent capture: '+side)
        for field, source in [('caller', CALLER), ('coordinator', 'controller-service/src/application/coordinator.rs')]:
            blob = subprocess.check_output(['git', 'show', f'{commit}:{source}'], cwd=root)
            require(row.get(field) == hashlib.sha256(blob).hexdigest(), 'coordinator parent source differs: '+side)
        expected = parent_dispatch(original(json.loads(data)), refactored=refactored)
        recorded = {field: row.get(field) for field in expected}
        # JSON distinguishes booleans from integers in the recorded branch and sender fields.
        require(json.dumps(recorded, sort_keys=True) == json.dumps(expected, sort_keys=True),
                'coordinator parent dispatch evidence differs: '+side)
