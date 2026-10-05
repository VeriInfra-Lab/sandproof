"""Docker -> OCI must preserve original verified blobs and all startup data."""
import copy
import gzip
import hashlib
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import Mock, patch

from tools.template_fetch import DOCKER_MANIFEST, OCI_MANIFEST, fetch
from tools.template_inputs import BlobReader, InvalidTemplate, inspect_oci, sha256
from tools.tests.test_template_inputs import archive_bytes


class ConversionTests(unittest.TestCase):
    def setUp(self):
        self.new_acquisition()

    def new_acquisition(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.layout = self.root / 'oci'
        (self.layout / 'blobs/sha256').mkdir(parents=True)
        layer = archive_bytes([('etc/value', b'original', tarfile.REGTYPE, '')])
        config = {'os': 'linux', 'architecture': 'amd64',
                  'config': {'User': '1000', 'WorkingDir': '/app', 'Env': ['X=literal'],
                             'Entrypoint': ['/entry'], 'Cmd': ['arg space'],
                             'OnBuild': ['RUN touch /must-not-execute']},
                  'rootfs': {'type': 'layers', 'diff_ids': ['sha256:' + hashlib.sha256(layer).hexdigest()]}}
        self.manifest = {'schemaVersion': 2, 'mediaType': DOCKER_MANIFEST,
            'config': self.add(json.dumps(config).encode(), 'application/vnd.docker.container.image.v1+json'),
            'layers': [self.add(gzip.compress(layer), 'application/vnd.docker.image.rootfs.diff.tar.gzip')]}
        self.write_manifest(self.manifest)
        (self.layout / 'oci-layout').write_text('{"imageLayoutVersion":"1.0.0"}')
        self.request = {'max_input_bytes': 100000, 'max_expanded_bytes': 100000, 'max_entries': 20}
        self.source = self.root / 'source.json'
        self.commands = Mock()

    def add(self, data, media):
        digest = hashlib.sha256(data).hexdigest()
        (self.layout / 'blobs/sha256' / digest).write_bytes(data)
        return {'digest': 'sha256:' + digest, 'size': len(data), 'mediaType': media}

    def write_manifest(self, manifest):
        self.raw = json.dumps(manifest).encode()
        descriptor = self.add(self.raw, manifest['mediaType'])
        self.digest = descriptor['digest']
        (self.layout / 'index.json').write_text(json.dumps({'schemaVersion': 2, 'manifests': [descriptor]}))

    def convert(self, args, *, changed_config=False):
        # Emulate the external converter, with independent intentional damage.
        self.assertEqual(args[1:3], ['image', 'mod'])
        self.assertEqual(args[3], 'ocidir://' + str(self.layout) + ':candidate')
        self.assertEqual(args[4:], ['--to-oci', '--replace'])
        descriptor = json.loads((self.layout / 'index.json').read_text())['manifests'][0]
        self.assertEqual(descriptor['digest'], self.digest)
        self.assertEqual(descriptor['annotations']['org.opencontainers.image.ref.name'], 'candidate')
        manifest = copy.deepcopy(self.manifest)
        manifest['mediaType'] = OCI_MANIFEST
        manifest['config']['mediaType'] = 'application/vnd.oci.image.config.v1+json'
        for layer in manifest['layers']:
            layer['mediaType'] = 'application/vnd.oci.image.layer.v1.tar+gzip'
        if changed_config:
            config = json.loads((self.layout / 'blobs/sha256' / manifest['config']['digest'][7:]).read_bytes())
            config['config']['User'] = 'root'
            manifest['config'] = self.add(json.dumps(config).encode(), 'application/vnd.oci.image.config.v1+json')
        self.write_manifest(manifest)

    def run_fetch(self):
        self.source.write_text(json.dumps({'image': 'docker.io/example/image@' + self.digest}))
        def capture(args, path, limit, commands):
            path.write_bytes(self.raw if 'manifest' in args else b'archive')
            return path.stat().st_size
        # The original layout is already populated; all hash and layer parsing
        # below are real. Only acquisition and converter execution are mocked.
        def tool_hash(path):
            return 'fixed' if path == Path('/tool') else sha256(path)
        with patch('tools.template_fetch.REGCTL_SHA256', 'fixed'), \
             patch('tools.template_fetch.sha256', side_effect=tool_hash), \
             patch('tools.template_fetch.capture', side_effect=capture), \
             patch('tools.template_fetch.extract_archive'):
            return fetch(self.source, self.root, self.layout, self.request, Path('/tool'), self.commands)

    def test_conversion_retains_verified_config_layers_and_source_provenance(self):
        original_digest = self.digest
        original_config = self.manifest['config']['digest']
        payload = [self.layout / 'blobs/sha256' / item['digest'][7:]
                   for item in [self.manifest['config'], *self.manifest['layers']]]
        stamps = [(p.stat().st_ino, p.stat().st_mtime_ns) for p in payload]
        self.commands.run.side_effect = self.convert
        result, image = self.run_fetch()
        self.assertEqual(result['selected_digest'], original_digest)
        self.assertNotEqual(result['converted_digest'], original_digest)
        self.assertEqual(image, inspect_oci(self.layout, max_bytes=100000, max_entries=20))
        self.assertEqual(image['image_digest'], result['converted_digest'])
        self.assertEqual(image['config_digest'], original_config)
        self.assertEqual(image['container_config']['OnBuild'], ['RUN touch /must-not-execute'])
        self.assertEqual(image['container_config']['Cmd'], ['arg space'])
        self.commands.run.assert_called_once()
        self.assertFalse((self.root / 'download.tar').exists())
        self.assertFalse((self.root / 'source-oci').exists())
        self.assertEqual(stamps, [(p.stat().st_ino, p.stat().st_mtime_ns) for p in payload])

    def test_original_docker_layout_is_rejected_without_explicit_conversion(self):
        with self.assertRaisesRegex(InvalidTemplate, 'unsupported OCI manifest'):
            inspect_oci(self.layout, max_bytes=100000, max_entries=20)

    def test_conversion_rehashes_both_sides_but_decompresses_unchanged_layers_once(self):
        path = self.layout / 'blobs/sha256' / self.manifest['layers'][0]['digest'][7:]
        self.commands.run.side_effect = self.convert
        with patch('tools.template_inputs.sha256', wraps=sha256) as hashes, \
             patch.object(BlobReader, 'verify', autospec=True, side_effect=BlobReader.verify) as streamed, \
             patch('gzip.GzipFile', wraps=gzip.GzipFile) as decompressor:
            self.run_fetch()
        # The first fingerprint is checked while decoding; conversion must
        # still perform an independent complete hash before reusing that result.
        self.assertEqual(sum(call.args[0].descriptor['digest'][7:] == path.name
                             for call in streamed.call_args_list), 1)
        self.assertEqual(sum(call.args[0] == path for call in hashes.call_args_list), 1)
        self.assertEqual(decompressor.call_count, 1)

    def test_converter_corrupting_previously_verified_bytes_is_rejected(self):
        def convert(args):
            self.convert(args)
            layer = self.manifest['layers'][0]
            (self.layout / 'blobs/sha256' / layer['digest'][7:]).write_bytes(b'x' * layer['size'])
        self.commands.run.side_effect = convert
        with self.assertRaisesRegex(InvalidTemplate, 'fingerprint'):
            self.run_fetch()

    def test_converted_encoding_and_diff_id_cannot_borrow_old_checks(self):
        def convert(args):
            self.convert(args)
            manifest = json.loads(self.raw)
            if change == 'encoding':
                manifest['layers'][0]['mediaType'] = 'application/vnd.oci.image.layer.v1.tar'
            elif change == 'diff_id':
                config = json.loads((self.layout / 'blobs/sha256' / manifest['config']['digest'][7:]).read_bytes())
                config['rootfs']['diff_ids'] = ['sha256:' + '0' * 64]
                manifest['config'] = self.add(json.dumps(config).encode(), manifest['config']['mediaType'])
            else:
                manifest['layers'][0]['urls'] = ['https://example.com/foreign']
            self.write_manifest(manifest)
        # Each subcase starts a fresh acquisition and private layout.
        for change, error in (('encoding', tarfile.ReadError), ('diff_id', InvalidTemplate),
                              ('foreign', InvalidTemplate)):
            with self.subTest(change=change):
                self.new_acquisition()
                self.commands.run.side_effect = convert
                with self.assertRaises(error):
                    self.run_fetch()

    def test_new_layer_digest_requires_full_path_validation(self):
        def convert(args):
            self.convert(args)
            manifest = json.loads(self.raw)
            layer = archive_bytes([('../escape', b'bad', tarfile.REGTYPE, '')])
            manifest['layers'] = [self.add(gzip.compress(layer), manifest['layers'][0]['mediaType'])]
            config = json.loads((self.layout / 'blobs/sha256' / manifest['config']['digest'][7:]).read_bytes())
            config['rootfs']['diff_ids'] = ['sha256:' + hashlib.sha256(layer).hexdigest()]
            manifest['config'] = self.add(json.dumps(config).encode(), manifest['config']['mediaType'])
            self.write_manifest(manifest)
        self.commands.run.side_effect = convert
        with self.assertRaisesRegex(InvalidTemplate, 'escapes root'):
            self.run_fetch()

    def test_converter_leaving_multiple_manifests_is_rejected(self):
        def convert(args):
            self.convert(args)
            index = json.loads((self.layout / 'index.json').read_text())
            index['manifests'].append(index['manifests'][0])
            (self.layout / 'index.json').write_text(json.dumps(index))
        self.commands.run.side_effect = convert
        with self.assertRaisesRegex(InvalidTemplate, 'expected one OCI image'):
            self.run_fetch()

    def test_failed_in_place_conversion_cannot_return_an_inspection(self):
        def convert(args):
            self.convert(args)
            raise TimeoutError('conversion interrupted')
        self.commands.run.side_effect = convert
        with self.assertRaisesRegex(TimeoutError, 'conversion interrupted'):
            self.run_fetch()

    def test_valid_but_changed_config_is_not_accepted_as_the_same_image(self):
        self.commands.run.side_effect = lambda args: self.convert(args, changed_config=True)
        with self.assertRaisesRegex(InvalidTemplate, 'changed image config or layers'):
            self.run_fetch()

    def test_corrupt_source_layer_is_rejected_before_converter_runs(self):
        layer = self.manifest['layers'][0]
        path = self.layout / 'blobs/sha256' / layer['digest'][7:]
        damaged = bytearray(path.read_bytes())
        damaged[4] ^= 1  # Valid gzip must still match the original encoded digest.
        path.write_bytes(damaged)
        with self.assertRaisesRegex(InvalidTemplate, 'fingerprint'):
            self.run_fetch()
        self.commands.run.assert_not_called()

    def test_foreign_layers_are_not_normalized_into_acceptance(self):
        self.manifest['layers'][0]['urls'] = ['https://example.com/foreign']
        self.write_manifest(self.manifest)
        with self.assertRaisesRegex(InvalidTemplate, 'tar/gzip'):
            self.run_fetch()
        self.commands.run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
