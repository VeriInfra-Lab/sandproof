"""Check coverage accounting for the original and proposed revision scopes."""
import hashlib
import json
from pathlib import Path
import unittest

from audit_service_sources import coverage


def jsonl(rows):
    return b''.join(json.dumps(row).encode() + b'\n' for row in rows)


class ServiceSourceCoverage(unittest.TestCase):
    def setUp(self):
        self.modules = {
            'SandboxModel/Shared.lean': '1' * 64,
            'SandproofRows/Types.lean': '2' * 64,
            'UnloadParentLink.lean': '3' * 64,
        }
        self.plan = json.dumps(dict(build_order=['SandboxModel.Shared', 'SandproofRows.Types',
                                                'UnloadParentLink'])).encode()
        self.requests = [
            dict(tool='lean_diagnostic_messages', arguments=dict(file_path=name, timeout_s=180),
                 expected_sha256=digest) for name, digest in self.modules.items()
        ]

    def rows(self, planned=False):
        sources = hashlib.sha256()
        paths = sorted(self.modules, key=Path) if planned else ['SandboxModel/Shared.lean']
        for name in paths:
            sources.update(len(name.encode()).to_bytes(8, 'big') + name.encode()
                           + bytes.fromhex(self.modules[name]))
        rows = []
        for request in self.requests:
            revision = dict(file_sha256=request['expected_sha256'], sources_sha256=sources.hexdigest())
            if planned:
                revision['source_plan_sha256'] = hashlib.sha256(self.plan).hexdigest()
            rows.append(dict(request=request, stale=False, revision=revision))
        return rows

    def read(self, rows, requests=None):
        source = self.requests if requests is None else requests
        return coverage(self.modules, self.plan, jsonl([*source, {'tool':'exit'}]), jsonl(rows))

    def test_per_file_replies_do_not_make_legacy_fingerprint_complete(self):
        result = self.read(self.rows())
        self.assertEqual(result['requested_files'], 3)
        self.assertEqual(result['fingerprinted_files'], 1)
        self.assertFalse(result['full_closure_bound'])
        self.assertEqual(set(result['omitted_files']), {'SandproofRows/Types.lean', 'UnloadParentLink.lean'})

    def test_plan_mode_binds_every_file_and_exact_plan(self):
        rows = self.rows(planned=True)
        result = self.read(rows)
        self.assertTrue(result['full_closure_bound'])
        self.assertEqual(result['fingerprinted_files'], 3)
        self.assertEqual(result['sources_sha256'], result['expected_full_sources_sha256'])
        rows[-1]['revision']['source_plan_sha256'] = '9' * 64
        with self.assertRaisesRegex(ValueError, 'same source plan'):
            self.read(rows)

    def test_plan_marker_cannot_authorize_legacy_digest(self):
        rows = self.rows(planned=True)
        legacy = self.rows()[0]['revision']['sources_sha256']
        for row in rows:
            row['revision']['sources_sha256'] = legacy
        with self.assertRaisesRegex(ValueError, 'wrong-scope'):
            self.read(rows)

    def test_requests_and_responses_must_cover_every_frozen_file_in_order(self):
        rows = self.rows()
        with self.assertRaisesRegex(ValueError, 'exact frozen view in order'):
            self.read(rows, self.requests[::-1])
        with self.assertRaisesRegex(ValueError, 'missing or extra'):
            self.read(rows[:-1])
        with self.assertRaisesRegex(ValueError, 'wrong-scope'):
            self.read(rows[::-1])

    def test_stale_or_mismatched_source_reply_is_refused(self):
        rows = self.rows(planned=True)
        rows[0]['stale'] = True
        with self.assertRaisesRegex(ValueError, 'stale, changed'):
            self.read(rows)
        rows[0]['stale'] = False
        rows[0]['revision']['file_sha256'] = '9' * 64
        with self.assertRaisesRegex(ValueError, 'stale, changed'):
            self.read(rows)


if __name__ == '__main__':
    unittest.main()
