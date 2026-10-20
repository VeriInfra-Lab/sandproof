"""Check source import admission, not Lua semantics or proof correctness."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    'redis_lua_source', ROOT / 'verification/experiments/redis-lua/source.py'
)
SOURCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SOURCE)


class LiteralSourceTests(unittest.TestCase):
    def setUp(self):
        self.raw = (ROOT / SOURCE.SCRIPTS).read_bytes()

    def test_real_inputs_are_preserved_as_bytes(self):
        _, scripts = SOURCE.source_record(ROOT)
        self.assertEqual(list(scripts), ['READ', 'CAS'])
        for raw in scripts.values():
            self.assertIn(raw, self.raw)
        self.assertIn(b"ARGV[5] == ARGV[7]", scripts['CAS'])
        self.assertIn(b"'revision', ARGV[4])", scripts['CAS'])

    def test_duplicate_or_reordered_declarations_rejected(self):
        with self.assertRaises(ValueError):
            SOURCE.scripts_from_bytes(self.raw + self.raw)
        wrong = self.raw.replace(b'const READ:', b'const TEMP:')
        with self.assertRaises(ValueError):
            SOURCE.scripts_from_bytes(wrong)

    def test_extra_rust_and_computed_string_rejected(self):
        for suffix in (b'fn extra() {}', b'const UNUSED: u8 = 1;'):
            with self.subTest(suffix=suffix), self.assertRaises(ValueError):
                SOURCE.scripts_from_bytes(self.raw + suffix)
        with self.assertRaises(ValueError):
            SOURCE.scripts_from_bytes(self.raw.replace(b'const READ: &str = r"',
                                                       b'const READ: &str = concat!(r"'))

    def test_unsupported_literal_domains_rejected(self):
        for wrong in (self.raw + b'\xff', self.raw.replace(b'local kind', b'local\0kind'),
                      b'x' * 16385):
            with self.subTest(wrong=wrong[:12]), self.assertRaises((ValueError, UnicodeError)):
                SOURCE.scripts_from_bytes(wrong)

    def test_source_edits_change_the_bound_identity(self):
        original = SOURCE.scripts_from_bytes(self.raw)
        edited = SOURCE.scripts_from_bytes(self.raw.replace(b'return 4', b'return 0'))
        self.assertEqual(original['READ'], edited['READ'])
        self.assertNotEqual(SOURCE.digest(original['CAS']), SOURCE.digest(edited['CAS']))

    def test_native_require_reaches_the_real_case_dispatch(self):
        from tools import proof_native_suite
        with mock.patch.object(proof_native_suite, 'native_case') as dispatch:
            proof_native_suite.main(['require', 'redis-lua'])
        dispatch.assert_called_once_with('redis-lua', proof_native_suite.ROOT, None)

    def test_stale_original_or_forged_reification_is_rejected(self):
        spec = importlib.util.spec_from_file_location(
            'redis_lua_check', ROOT / 'verification/experiments/redis-lua/check.py'
        )
        module = importlib.util.module_from_spec(spec)
        with mock.patch.dict('sys.modules', {'source': SOURCE}):
            spec.loader.exec_module(module)
        names = [SOURCE.SCRIPTS, SOURCE.STORE,
                 Path('verification/experiments/redis-lua/source-map.json'),
                 Path('SandboxModel/CenterStore/Lua/RawSource.lean')]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in names:
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                (root / name).write_bytes((ROOT / name).read_bytes())
            (root / SOURCE.SCRIPTS).write_bytes(self.raw.replace(b'return 4', b'return 0'))
            with self.assertRaisesRegex(ValueError, 'original Lua or Rust caller changed'):
                module.check(root)
            (root / SOURCE.SCRIPTS).write_bytes(self.raw)
            raw = root / 'SandboxModel/CenterStore/Lua/RawSource.lean'
            raw.write_text(raw.read_text() + '\n-- forged trailing material\n')
            with self.assertRaisesRegex(ValueError, 'raw input differs'):
                module.check(root)
            frozen = json.loads((root / names[2]).read_text())
            frozen['scope'] = 'entire deployed service verified'
            (root / names[2]).write_text(json.dumps(frozen))
            with self.assertRaisesRegex(ValueError, 'unreviewed Lua batch scope'):
                module.check(root)


if __name__ == '__main__':
    unittest.main()
