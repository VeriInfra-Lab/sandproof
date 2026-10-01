import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from tools.template_fetch import DOCKER_MANIFEST, OCI_INDEX, OCI_MANIFEST, capture, fetch, reference, select_manifest
from tools.template_inputs import InvalidTemplate
from tools.template_worker import Commands


class PublicFetchTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def test_public_references_without_credentials_or_options(self):
        digest = 'a' * 64
        for repo in ('docker.io/library/debian', 'ghcr.io/org/sub/image', 'quay.io/image'):
            self.assertEqual(reference(repo + '@sha256:' + digest), (repo, 'sha256:' + digest))
        for value in ('https://docker.io/library/debian',
                      'localhost/image', '127.0.0.1/image', '10.0.0.1/image', 'host.local/image',
                      'docker.io:5000/image', 'user:password@docker.io/image', 'docker.io/../image',
                      'docker.io/image\nRUN bad', '--host x/image', 'docker.io/image?x=1',
                      'docker.io/image#fragment', 'docker.io/image;echo x', 'docker.io//image'):
            with self.subTest(value=value), self.assertRaises(InvalidTemplate):
                reference(value + '@sha256:' + digest)
        for value, expected in (
                ('ubuntu:24.04', ('docker.io/library/ubuntu', '24.04')),
                ('ubuntu', ('docker.io/library/ubuntu', 'latest')),
                ('org/image', ('docker.io/org/image', 'latest')),
                ('ghcr.io/org/image:v1', ('ghcr.io/org/image', 'v1'))):
            self.assertEqual(reference(value), expected)
        for value in ('localhost/image', '10.0.0.1/image', 'host.local/image',
                      'registry:5000/image', 'docker.io/image:bad/tag', 'ubuntu@sha256:bad'):
            with self.subTest(value=value), self.assertRaises(InvalidTemplate):
                reference(value)

    def test_platform_selection_does_not_guess_ambiguous_or_unsupported_image(self):
        digest = 'sha256:' + 'a' * 64
        selected = {'mediaType': OCI_MANIFEST, 'digest': digest, 'size': 120,
                    'platform': {'os': 'linux', 'architecture': 'amd64'}}
        index = {'schemaVersion': 2, 'mediaType': OCI_INDEX, 'manifests': [selected]}
        self.assertEqual(select_manifest(index, 'parent'), digest)
        other = copy.deepcopy(selected); other['platform']['architecture'] = 'arm64'
        index['manifests'].append(other)
        self.assertEqual(select_manifest(index, 'parent'), digest)
        for manifests in ([selected, selected], [other], [], [dict(selected, size=True)],
                          [dict(selected, digest='sha256:no')], [dict(selected, platform=None)],
                          [dict(selected, mediaType='application/vnd.docker.distribution.manifest.v1+json')],
                          [dict(selected, platform={'os':'linux','architecture':'amd64','variant':'v3'})]):
            with self.subTest(manifests=manifests), self.assertRaises(InvalidTemplate):
                select_manifest(dict(index, manifests=manifests), 'parent')

    def test_docker_manifest_and_index_select_without_schema1_or_platform_guessing(self):
        digest = 'sha256:' + 'd' * 64
        manifest = {'schemaVersion': 2, 'mediaType': DOCKER_MANIFEST}
        self.assertEqual(select_manifest(manifest, digest), digest)
        descriptor = {'mediaType': DOCKER_MANIFEST, 'digest': digest, 'size': 100,
                      'platform': {'os': 'linux', 'architecture': 'amd64'}}
        self.assertEqual(select_manifest({'schemaVersion': 2, 'mediaType': OCI_INDEX,
                                         'manifests': [descriptor]}, 'parent'), digest)
        with self.assertRaises(InvalidTemplate):
            select_manifest(dict(manifest, schemaVersion=1), digest)

    def test_binary_capture_exact_limit_and_separate_bounded_log(self):
        log = io.BytesIO()
        command = Commands(time.monotonic()+5, log, 6)
        size = capture([sys.executable, '-c', 'import os;os.write(1,b"\\x00\\xff12");os.write(2,b"logged")'],
                       self.root/'download', 4, command)
        self.assertEqual(size, 4)
        self.assertEqual((self.root/'download').read_bytes(), b'\0\xff12')
        self.assertEqual(log.getvalue(), b'logged')
        self.assertEqual(command.budget.remaining, 0)

    def test_output_and_log_overflow_nonzero_and_closed_pipe_deadline(self):
        scripts = [
            ('import os;os.write(1,b"x"*5)', InvalidTemplate),
            ('import os;os.write(2,b"x"*5)', InvalidTemplate),
            ('raise SystemExit(17)', InvalidTemplate),
            # Both pipes close before process exit. Still enforce the deadline.
            ('import os,time;os.close(1);os.close(2);time.sleep(30)', TimeoutError),
        ]
        for i,(script,error) in enumerate(scripts):
            log=io.BytesIO();command=Commands(time.monotonic()+0.5,log,4)
            with self.subTest(script=script),self.assertRaises(error):
                capture([sys.executable,'-c',script],self.root/str(i),4,command)
            self.assertLessEqual((self.root/str(i)).stat().st_size,4)
            self.assertLessEqual(len(log.getvalue()),4)

    def test_source_digest_mismatch_stops_before_any_layer_fetch(self):
        source=self.root/'source.json';source.write_text(json.dumps({'image':'docker.io/library/debian@sha256:'+'a'*64}))
        def fake_capture(args, output, limit, commands):
            output.write_bytes(b'{"schemaVersion":2}')
            return output.stat().st_size
        with patch('tools.template_fetch.sha256',side_effect=['fixed','wrong']), \
             patch('tools.template_fetch.REGCTL_SHA256','fixed'), \
             patch('tools.template_fetch.capture',side_effect=fake_capture) as calls:
            with self.assertRaisesRegex(InvalidTemplate,'digest mismatch'):
                fetch(source,self.root,self.root/'oci',{'max_input_bytes':1024,'max_entries':100},Path('/tool'),None)
            self.assertEqual(calls.call_count,1)

    def test_downloaded_descriptor_must_match_the_pinned_source(self):
        manifest={'schemaVersion':2,'mediaType':OCI_MANIFEST}
        data=json.dumps(manifest).encode();digest=hashlib.sha256(data).hexdigest()
        source=self.root/'source.json';source.write_text(json.dumps({'image':'docker.io/library/debian@sha256:'+digest}))
        def fake_capture(args, output, limit, commands):
            output.write_bytes(data if 'manifest' in args else b'archive')
            return output.stat().st_size
        def fake_extract(source,layout,**limits):
            layout.mkdir();(layout/'index.json').write_text(json.dumps({'manifests':[{'mediaType':OCI_MANIFEST,'digest':'sha256:'+'b'*64}]}))
        with patch('tools.template_fetch.sha256',side_effect=['fixed',digest]), \
             patch('tools.template_fetch.REGCTL_SHA256','fixed'), \
             patch('tools.template_fetch.capture',side_effect=fake_capture), \
             patch('tools.template_fetch.extract_archive',side_effect=fake_extract):
            with self.assertRaisesRegex(InvalidTemplate,'selected digest'):
                fetch(source,self.root,self.root/'oci',{'max_input_bytes':1024,'max_entries':100},Path('/tool'),None)

    def test_mutable_tag_is_resolved_once_then_export_is_pinned(self):
        manifest={'schemaVersion':2,'mediaType':OCI_MANIFEST}
        data=json.dumps(manifest).encode();digest='sha256:'+hashlib.sha256(data).hexdigest()
        source=self.root/'source.json';source.write_text(json.dumps({'image':'ubuntu:24.04'}))
        def fake_capture(args, output, limit, commands):
            output.write_bytes(data if 'manifest' in args else b'archive')
            return output.stat().st_size
        def fake_extract(source,layout,**limits):
            layout.mkdir();(layout/'index.json').write_text(json.dumps({'manifests':[{'mediaType':OCI_MANIFEST,'digest':digest}]}))
        with patch('tools.template_fetch.sha256',side_effect=['fixed',digest[7:]]), \
             patch('tools.template_fetch.REGCTL_SHA256','fixed'), \
             patch('tools.template_fetch.capture',side_effect=fake_capture) as calls, \
             patch('tools.template_fetch.extract_archive',side_effect=fake_extract):
            provenance=fetch(source,self.root,self.root/'oci',{'max_input_bytes':1024,'max_entries':100},Path('/tool'),None)
        self.assertEqual(calls.call_args_list[0].args[0][3], 'docker.io/library/ubuntu:24.04')
        self.assertEqual(calls.call_args_list[1].args[0][3], 'docker.io/library/ubuntu@'+digest)
        self.assertEqual(provenance['source_digest'], digest)
        self.assertEqual(provenance['selected_digest'], digest)


if __name__=='__main__':
    unittest.main()
