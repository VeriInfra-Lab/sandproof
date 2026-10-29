"""Reuse the already reviewed Vec binding and fixed two-toolchain std sources."""
import importlib.util
from pathlib import Path
import re

HERE = Path('verification/experiments/stage1-protocol/native-readiness')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def check(root):
    parent = module('native_completion_existing_std', root/HERE.parent/'inbox-unload-resolution/library_binding.py')
    syntax = module('native_completion_existing_binding_syntax', root/HERE.parent/'inbox-renewal-session/library_binding.py')
    reviewed = parent.check(root)
    current = (root/HERE/'FunsExternal.lean.in').read_text()
    original = (root/HERE.parent/'inbox-unload-resolution/FunsExternal.lean.in').read_text()
    symbol = 'alloc.vec.Vec.is_empty'
    if set(re.findall(r'^def (\S+)', current, re.M)) != {symbol}:
        raise ValueError('unreviewed native completion primitive')
    if syntax.body(current, symbol) != syntax.body(original, symbol):
        raise ValueError('registered Vec primitive body differs')
    return dict(symbols=[symbol], fixed_std_sources=reviewed['sources'],
                original_library_implementation_proved=False, new_trusted_contracts=[])
