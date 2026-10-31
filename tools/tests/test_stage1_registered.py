"""Discover the original registered-entry regressions in the functional gate.

These tests use synthetic evidence and do not run a compiler or Lean service.
"""
import importlib.util
from pathlib import Path


def load_tests(loader, tests, pattern):
    path = Path(__file__).resolve().parents[2]/'verification/experiments/stage1-protocol/test_registered.py'
    spec = importlib.util.spec_from_file_location('registered_entry_regressions', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    tests.addTests(loader.loadTestsFromModule(module))
    return tests
