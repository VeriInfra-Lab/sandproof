"""Exercise real worker acquisition/inspection up to the external unpack boundary."""
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import tarfile
import unittest
from unittest.mock import patch

from tools import template_worker as worker
from tools.template_inputs import InvalidTemplate, sha256
from tools.tests import test_template_conversion as fixtures


class ReachedUnpack(RuntimeError):
    pass


class WorkerAcquisitionTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.ConversionTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.original_layout = self.fixture.layout
        self.work = self.root / 'worker'
        self.source = self.root / 'input'
        self.unpacked = []
        self.converted = []
        self.request = dict(self.fixture.request, kind='oci-reference', disk_mib=512,
                            min_free_mib=16, timeout_seconds=5, max_log_bytes=65536)

    def native_oci(self):
        manifest = self.fixture.manifest
        manifest['mediaType'] = fixtures.OCI_MANIFEST
        manifest['config']['mediaType'] = 'application/vnd.oci.image.config.v1+json'
        manifest['layers'][0]['mediaType'] = 'application/vnd.oci.image.layer.v1.tar+gzip'
        self.fixture.write_manifest(manifest)

    def acquire(self, *, damage_conversion=False):
        self.source.write_text(json.dumps({'image': 'docker.io/example/image@' + self.fixture.digest}))
        raw = self.fixture.raw
        def capture(args, path, limit, commands):
            path.write_bytes(raw if 'manifest' in args else b'synthetic transport')
            return path.stat().st_size
        def extract(source, layout, **limits):
            shutil.copytree(self.original_layout, layout)
        def command(args, **kwargs):
            if args[0] == '/tool':
                self.converted.append(args)
                self.fixture.root, self.fixture.layout = self.work, self.work / 'oci'
                self.fixture.convert(args)
                if damage_conversion:
                    layer = self.fixture.manifest['layers'][0]
                    (self.fixture.layout / 'blobs/sha256' / layer['digest'][7:]).write_bytes(b'x' * layer['size'])
            else:
                self.assertEqual(args[:2], ['/never-execute-umoci', 'unpack'])
                self.unpacked.append(args)
                raise ReachedUnpack()
        def tool_hash(path):
            return 'fixed' if path == Path('/tool') else sha256(path)
        real_read = Path.read_text
        def read_text(path, *args, **kwargs):
            if str(path) == '/proc/sys/kernel/random/boot_id':
                return 'synthetic-boot'
            return real_read(path, *args, **kwargs)
        tools = {'regctl': Path('/tool'), 'umoci': Path('/never-execute-umoci')}
        # The host/transport/umoci boundaries are synthetic. Both Docker and OCI
        # validation, config interpretation and compressed bytes are real.
        with patch.object(worker.os, 'geteuid', return_value=0), \
             patch.object(worker.sys, 'platform', 'linux'), \
             patch.object(Path, 'is_block_device', return_value=True), \
             patch.object(Path, 'read_text', read_text), \
             patch.object(worker, 'check_tools', return_value=tools), \
             patch.object(worker, 'sha256', return_value='fixed'), \
             patch.object(worker.Commands, 'run', side_effect=command), \
             patch('tools.template_fetch.REGCTL_SHA256', 'fixed'), \
             patch('tools.template_fetch.sha256', side_effect=tool_hash), \
             patch('tools.template_fetch.capture', side_effect=capture), \
             patch('tools.template_fetch.extract_archive', side_effect=extract), \
             patch.object(worker, 'extract_archive', side_effect=extract):
            return worker.build(self.work, self.source, self.request, {}, guest_boot_id='synthetic-boot')

    def test_docker_checks_source_and_converted_layers_once_each_before_unpack(self):
        with patch('gzip.GzipFile', wraps=gzip.GzipFile) as decoder:
            with self.assertRaises(ReachedUnpack):
                self.acquire()
        self.assertEqual(len(self.converted), 1)
        self.assertEqual(len(self.unpacked), 1)
        self.assertEqual(decoder.call_count, 2)

    def test_native_public_oci_checks_layers_once_before_unpack(self):
        self.native_oci()
        with patch('gzip.GzipFile', wraps=gzip.GzipFile) as decoder:
            with self.assertRaises(ReachedUnpack):
                self.acquire()
        self.assertEqual(self.converted, [])
        self.assertEqual(len(self.unpacked), 1)
        self.assertEqual(decoder.call_count, 1)

    def test_corrupt_converted_blob_is_rejected_before_unpack(self):
        with self.assertRaisesRegex(InvalidTemplate, 'fingerprint'):
            self.acquire(damage_conversion=True)
        self.assertEqual(len(self.converted), 1)
        self.assertEqual(self.unpacked, [])

    def test_native_public_and_uploaded_oci_still_reject_corrupt_layers(self):
        for kind in ('oci-reference', 'oci'):
            with self.subTest(kind=kind):
                # Each job has a fresh private work directory; no inspection cache.
                self.work = self.root / ('worker-' + kind)
                self.request['kind'] = kind
                self.native_oci()
                layer = self.fixture.manifest['layers'][0]
                (self.original_layout / 'blobs/sha256' / layer['digest'][7:]).write_bytes(b'x' * layer['size'])
                with self.assertRaisesRegex(InvalidTemplate, 'fingerprint'):
                    self.acquire()
                self.assertEqual(self.unpacked, [])

    def test_native_public_oci_expanded_and_entry_budgets_are_still_enforced(self):
        for name, payload, expanded, entries in (('bytes', b'x' * 9000, 10240, 20),
                                                ('entries', b'x', 100000, 1)):
            with self.subTest(limit=name):
                self.work = self.root / ('worker-' + name)
                self.native_oci()
                layer = fixtures.archive_bytes([('a', payload, tarfile.REGTYPE, ''),
                                                ('b', b'x', tarfile.REGTYPE, '')])
                manifest = self.fixture.manifest
                config = json.loads((self.original_layout / 'blobs/sha256' /
                                     manifest['config']['digest'][7:]).read_bytes())
                config['rootfs']['diff_ids'] = ['sha256:' + hashlib.sha256(layer).hexdigest()]
                manifest['config'] = self.fixture.add(json.dumps(config).encode(),
                                                      'application/vnd.oci.image.config.v1+json')
                manifest['layers'] = [self.fixture.add(gzip.compress(layer),
                                                      'application/vnd.oci.image.layer.v1.tar+gzip')]
                self.fixture.write_manifest(manifest)
                self.request.update(max_expanded_bytes=expanded, max_entries=entries)
                with self.assertRaises(InvalidTemplate):
                    self.acquire()
                self.assertEqual(self.unpacked, [])


if __name__ == '__main__':
    unittest.main()
