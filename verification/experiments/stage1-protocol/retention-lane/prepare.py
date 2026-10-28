"""Prepare thin roots and real mutations of the adopted production implementation."""
from hashlib import sha256
from pathlib import Path
import importlib.util
import json


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


views = module('adopted_source_views', Path(__file__).parents[1] / 'source_views.py')
sha = views.sha

HERE = Path('verification/experiments/stage1-protocol/retention-lane')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'
MUTATIONS = {
    'ignore-successor': ('if self.current_window != successor {\n            return false;\n        }',
        'if self.current_window != successor {\n            return true;\n        }'),
    'allow-pending': ('last.status == sandproof_protocol::lifecycle::TerminalStatus::Cleaned\n                    && last.matches_request(&last.request)',
        '({\n                    let _ = last.status == sandproof_protocol::lifecycle::TerminalStatus::Cleaned;\n                    true\n                }) && last.matches_request(&last.request)'),
    'ignore-context': ('&& last.request.validate(context).is_ok()',
        '&& {\n                        let _ = last.request.validate(context);\n                        true\n                    }'),
    'omit-slot': ('                    && last.request.retention.slot == self.slot\n', ''),
    'omit-window': ('                    && last.request.retention.window.get() == floor\n', ''),
    'oversize-encoding': ('Ok(bytes) => bytes.len() <= 8192,', 'Ok(bytes) => bytes.len() <= 8193,'),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')



def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    if variant in MUTATIONS:
        mutation = views.edit(project, SOURCE, [MUTATIONS[variant]])
    views.append(project, SOURCE, (project/HERE/'roots.rs.in').read_text())
    if variant not in ('original', 'restored'):
        views.append(project, SOURCE, (project/HERE/'generic-root.rs.in').read_text())
    views.append(project, SHIM, (project/HERE.parent/'terminal-binding/roots.rs.in').read_text())
    return views.finish(project, before, mutation)
