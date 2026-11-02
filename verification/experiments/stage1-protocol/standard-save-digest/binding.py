"""Reuse the single adopted SHA leaf admission without a second rule set."""
from pathlib import Path
import importlib.util
provider=Path(__file__).resolve().parent.parent/'standard-digest/binding.py'
spec=importlib.util.spec_from_file_location('save_unique_sha_admission',provider)
original=importlib.util.module_from_spec(spec)
spec.loader.exec_module(original)
admit=original.admit
