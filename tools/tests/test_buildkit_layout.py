"""Private BuildKit store linking still feeds the complete OCI validator."""
import gzip
import hashlib
import json
import shutil
import unittest

from tools.template_inputs import InvalidTemplate, inspect_oci
from tools.template_worker import buildkit_layout
from tools.tests import test_template_conversion as fixtures


class BuildkitLayoutTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.ConversionTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        manifest = self.fixture.manifest
        manifest['mediaType'] = fixtures.OCI_MANIFEST
        manifest['config']['mediaType'] = 'application/vnd.oci.image.config.v1+json'
        manifest['layers'][0]['mediaType'] = 'application/vnd.oci.image.layer.v1.tar+gzip'
        self.fixture.write_manifest(manifest)
        self.descriptor = json.loads((self.fixture.layout / 'index.json').read_text())['manifests'][0]
        self.cache = self.root / 'cache'
        self.store = self.cache / 'runc-overlayfs/content/blobs/sha256'
        shutil.copytree(self.fixture.layout / 'blobs/sha256', self.store)
        self.metadata = self.root / 'metadata.json'
        self.exported = {'containerimage.descriptor': self.descriptor,
                         'containerimage.digest': self.descriptor['digest'],
                         'containerimage.config.digest': manifest['config']['digest']}
        self.save_metadata(self.exported)
        self.output = self.root / 'linked'

    def save_metadata(self, exported):
        self.metadata.write_text(json.dumps(exported))

    def link(self, maximum=100000):
        buildkit_layout(self.cache, self.metadata, self.output, max_bytes=maximum)

    def inspect(self, **limits):
        return inspect_oci(self.output, max_bytes=limits.get('max_bytes', 100000),
                           max_entries=limits.get('max_entries', 20))

    def test_links_only_selected_image_and_preserves_exact_descriptor_config_and_blobs(self):
        original = inspect_oci(self.fixture.layout, max_bytes=100000, max_entries=20)
        self.link()
        self.assertEqual(self.inspect(), original)
        self.assertEqual(json.loads((self.output / 'index.json').read_text())['manifests'], [self.descriptor])
        expected = [self.descriptor, self.fixture.manifest['config'], *self.fixture.manifest['layers']]
        self.assertEqual({p.name for p in (self.output / 'blobs/sha256').iterdir()},
                         {entry['digest'][7:] for entry in expected})
        for entry in expected:
            source, target = self.store / entry['digest'][7:], self.output / 'blobs/sha256' / entry['digest'][7:]
            self.assertEqual(source.stat().st_ino, target.stat().st_ino)
            self.assertEqual(hashlib.sha256(target.read_bytes()).hexdigest(), entry['digest'][7:])
        # Independent links retain the exact bytes after this job's cache goes away.
        shutil.rmtree(self.cache)
        self.assertEqual(self.inspect(), original)

    def test_native_and_fuse_stores_are_selected_without_changing_image(self):
        for name in ('runc-native', 'runc-fuse-overlayfs'):
            with self.subTest(snapshotter=name):
                old = self.cache / 'runc-overlayfs'
                old.rename(self.cache / name)
                self.link()
                self.assertEqual(self.inspect()['image_digest'], self.descriptor['digest'])
                shutil.rmtree(self.output)
                (self.cache / name).rename(old)

    def test_ambiguous_missing_and_symlinked_stores_are_rejected(self):
        shutil.copytree(self.cache / 'runc-overlayfs', self.cache / 'runc-native')
        with self.assertRaisesRegex(InvalidTemplate, 'one private'):
            self.link()
        shutil.rmtree(self.cache / 'runc-native')
        for relative in ('runc-overlayfs', 'runc-overlayfs/content',
                         'runc-overlayfs/content/blobs', 'runc-overlayfs/content/blobs/sha256'):
            with self.subTest(path=relative):
                path = self.cache / relative
                parked = self.root / 'parked'
                path.rename(parked)
                path.symlink_to(parked, target_is_directory=True)
                with self.assertRaises(InvalidTemplate):
                    self.link()
                path.unlink()
                parked.rename(path)
        shutil.rmtree(self.cache / 'runc-overlayfs')
        with self.assertRaisesRegex(InvalidTemplate, 'one private'):
            self.link()

    def test_export_metadata_must_bind_a_single_oci_manifest_and_config(self):
        cases = [[], {}, self.exported | {'containerimage.digest': 'sha256:' + '0' * 64},
                 self.exported | {'containerimage.config.digest': 'sha256:' + '0' * 64}]
        for change in ({'mediaType': 'application/vnd.oci.image.index.v1+json'},
                       {'digest': 'sha256:../../escape'}, {'size': True},
                       {'urls': ['https://example.invalid/layer']}):
            modified = self.exported | {'containerimage.descriptor': self.descriptor | change}
            if 'digest' in change:
                modified['containerimage.digest'] = change['digest']
            cases.append(modified)
        for exported in cases:
            with self.subTest(exported=exported):
                self.save_metadata(exported)
                with self.assertRaises(InvalidTemplate):
                    self.link()
                if self.output.exists():
                    shutil.rmtree(self.output)

    def test_missing_symlink_and_truncated_blob_fail_before_inspection(self):
        path = self.store / self.fixture.manifest['layers'][0]['digest'][7:]
        saved = path.read_bytes()
        path.unlink()
        with self.assertRaises(FileNotFoundError):
            self.link()
        shutil.rmtree(self.output)
        path.symlink_to(self.fixture.layout / 'blobs/sha256' / path.name)
        with self.assertRaises(InvalidTemplate):
            self.link()
        shutil.rmtree(self.output)
        path.unlink()
        path.write_bytes(saved[:-1])
        with self.assertRaises(InvalidTemplate):
            self.link()

    def test_changed_same_length_layer_is_rehashed_and_cumulative_limits_remain(self):
        self.link()
        with self.assertRaises(InvalidTemplate):
            self.inspect(max_bytes=10239)
        with self.assertRaises(InvalidTemplate):
            self.inspect(max_entries=0)
        path = self.store / self.fixture.manifest['layers'][0]['digest'][7:]
        damaged = bytearray(path.read_bytes())
        damaged[4] ^= 1  # A valid gzip header change leaves the decoded tar intact.
        path.write_bytes(damaged)
        with self.assertRaisesRegex(InvalidTemplate, 'fingerprint'):
            self.inspect()
        path.write_bytes(b'x' * path.stat().st_size)
        with self.assertRaises(gzip.BadGzipFile):
            self.inspect()

    def test_manifest_corruption_and_oversized_descriptors_are_rejected(self):
        with self.assertRaises(InvalidTemplate):
            self.link(maximum=1)
        shutil.rmtree(self.output)
        path = self.store / self.descriptor['digest'][7:]
        path.write_bytes(b'x' * path.stat().st_size)
        with self.assertRaisesRegex(InvalidTemplate, 'fingerprint'):
            self.link()

    def test_duplicate_layer_links_remain_subject_to_each_occurrence_budget(self):
        manifest = self.fixture.manifest
        config = json.loads((self.fixture.layout / 'blobs/sha256' / manifest['config']['digest'][7:]).read_bytes())
        config['rootfs']['diff_ids'] *= 2
        manifest['config'] = self.fixture.add(json.dumps(config).encode(), 'application/vnd.oci.image.config.v1+json')
        manifest['layers'] *= 2
        self.fixture.write_manifest(manifest)
        shutil.copytree(self.fixture.layout / 'blobs/sha256', self.store, dirs_exist_ok=True)
        descriptor = json.loads((self.fixture.layout / 'index.json').read_text())['manifests'][0]
        self.save_metadata({'containerimage.descriptor': descriptor, 'containerimage.digest': descriptor['digest'],
                            'containerimage.config.digest': manifest['config']['digest']})
        self.link()
        self.inspect(max_bytes=20480, max_entries=2)
        for limits in ({'max_bytes': 20479}, {'max_entries': 1}):
            with self.subTest(limits=limits), self.assertRaises(InvalidTemplate):
                self.inspect(**limits)


if __name__ == '__main__':
    unittest.main()
