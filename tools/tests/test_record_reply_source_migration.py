import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
HERE = ROOT/'verification/experiments/stage1-protocol/record-reply'
spec = importlib.util.spec_from_file_location('record_reply_source_migration_tests', HERE/'support.py')
support = importlib.util.module_from_spec(spec)
spec.loader.exec_module(support)


class RecordReplySourceMigration(unittest.TestCase):
    def test_current_sources_still_use_the_historical_index_and_original_boundaries(self):
        data = support.inventory()
        index = json.loads((HERE/'generated-index.json').read_bytes())
        self.assertNotEqual(index['actual_source_sha256'], data['source_inputs'][support.MODEL])
        self.assertEqual(tuple(index['variants']), support.CASES)

    def test_changed_reply_helper_is_rejected_even_when_the_file_has_the_expected_path(self):
        data = json.loads((HERE/'inventory.json').read_bytes())
        index = json.loads((HERE/'generated-index.json').read_bytes())
        read = Path.read_bytes

        def changed(path):
            content = read(path)
            if path == ROOT/support.MODEL:
                return content.replace(b'recorded.matches(intent)', b'recorded.terminal', 1)
            return content

        with patch.object(Path, 'read_bytes', changed):
            with self.assertRaisesRegex(ValueError, 'actual reply source declaration changed'):
                support.check_source_migration(data, index)

    def test_only_current_variant_declaration_locations_can_differ(self):
        data = json.loads((HERE/'inventory.json').read_bytes())
        index = json.loads((HERE/'generated-index.json').read_bytes())
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory)
            for name in (support.MODEL, support.SERVER):
                destination = project/name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT/name, destination)
            current = (project/support.MODEL).read_bytes()
            for variant in support.CASES:
                (project/support.MODEL).write_bytes(current)
                support.mutate(project, data['variants'][variant]['mutation'])
                for name, digest in index['variants'][variant]['generated_sha256'].items():
                    with self.subTest(variant=variant, generated=name):
                        old = (ROOT/support.PUBLISHED/variant/'SandproofStage1Request'/name).read_bytes()
                        candidate = support.relocated_output(project, old, name)
                        support.check_generated_output(project, name, candidate, old, digest)
                        self.assertNotEqual(candidate, old)
                        for wrong in (old, candidate.replace(b'server.lifecycle.RecordReplyError', b'other.Error', 1),
                                      candidate.replace(b"'sandbox-internal/src/server/lifecycle.rs'", b"'other.rs'", 1)):
                            with self.assertRaisesRegex(ValueError, 'beyond reviewed source spans'):
                                support.check_generated_output(project, name, wrong, old, digest)


if __name__ == '__main__':
    unittest.main()
