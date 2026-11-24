"""Check local Rust source admission without executing rustup or a compiler."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools import stage1_host


class RustSourceAdmissionTests(unittest.TestCase):
    def test_two_pinned_toolchains_require_every_original_source_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            rustup_home = root / 'rustup'
            target = 'x86_64-unknown-linux-gnu'
            pin = root / stage1_host.SERDE_RUST_SOURCE_PIN
            pin.parent.mkdir(parents=True)
            original = Path(stage1_host.__file__).resolve().parents[1] / stage1_host.SERDE_RUST_SOURCE_PIN
            inputs = json.loads(original.read_text())['library_inputs']['standard_library']
            sources = {}
            for channel, files in inputs.items():
                sysroot = rustup_home / 'toolchains' / f'{channel}-{target}'
                compiler = sysroot / 'bin/rustc'
                compiler.parent.mkdir(parents=True)
                compiler.touch(mode=0o700)
                for relative in files:
                    path = sysroot / 'lib/rustlib/src/rust/library' / relative
                    path.parent.mkdir(parents=True, exist_ok=True)
                    material = f'fixture bytes for {channel}/{relative}'.encode()
                    path.write_bytes(material)
                    files[relative] = hashlib.sha256(material).hexdigest()
                    sources[(channel, relative)] = path
            pin.write_text(json.dumps({'library_inputs': {'standard_library': inputs}}))
            complete = stage1_host.pinned_rust_sources(root, rustup_home, target)
            self.assertEqual({channel: row['sources'] for channel, row in complete.items()}, inputs)
            needed = sources[('nightly-2026-02-07', 'std/src/io/mod.rs')]
            needed.unlink()
            with self.assertRaisesRegex(ValueError, 'missing locally: nightly-2026-02-07/std/src/io/mod.rs'):
                stage1_host.pinned_rust_sources(root, rustup_home, target)
            needed.write_text('replacement Rust source')
            with self.assertRaisesRegex(ValueError, 'differs from the serde binding: nightly-2026-02-07/std/src/io/mod.rs'):
                stage1_host.pinned_rust_sources(root, rustup_home, target)
