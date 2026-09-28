"""Offline builder input and staging boundaries; no root, images or VM required."""

import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import build_base_guest as builder


class BaseGuestTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.work = Path(self.temporary.name)
        self.source = self.work / "source"
        self.source.write_bytes(b"fixed input")
        self.expected = {"bytes": 11, "sha256": hashlib.sha256(b"fixed input").hexdigest()}

    def test_copy_is_private_and_must_match_fingerprint(self):
        destination = self.work / "verified"
        builder.verified_copy(self.source, destination, self.expected)
        self.source.write_bytes(b"later input")
        self.assertEqual(destination.read_bytes(), b"fixed input")
        self.assertEqual(destination.stat().st_mode & 0o777, 0o400)
        with self.assertRaisesRegex(ValueError, "fingerprint"):
            builder.verified_copy(self.source, self.work / "wrong", self.expected)

    def test_copy_rejects_wrong_size_symlink_and_existing_destination(self):
        alias = self.work / "alias"
        alias.symlink_to(self.source)
        for source, expected in ((alias, self.expected), (self.source, dict(self.expected, bytes=12))):
            with self.assertRaises(ValueError):
                builder.verified_copy(source, self.work / "unused", expected)
        with self.assertRaises(FileExistsError):
            builder.verified_copy(self.source, self.source, self.expected)
        self.assertEqual(self.source.read_bytes(), b"fixed input")

    def test_write_rejects_parent_escape_and_final_symlink(self):
        root = self.work / "root"
        root.mkdir()
        (root / "escape").symlink_to(self.work, target_is_directory=True)
        (root / "alias").symlink_to(self.source)
        for name in ("/escape/source", "/../source", "/alias"):
            with self.assertRaises(ValueError):
                builder.guest_destination(root, name)
        self.assertEqual(self.source.read_bytes(), b"fixed input")

    def test_replacing_guest_symlink_never_follows_final_target(self):
        root = self.work / "root"
        root.mkdir()
        (root / "alias").symlink_to(self.source)
        target = builder.guest_destination(root, "/alias", replace_link=True)
        target.unlink()
        target.symlink_to("/dev/null")
        self.assertEqual(self.source.read_bytes(), b"fixed input")

    def test_guest_missing_shadow_cannot_pass_by_empty_debugfs_output(self):
        profile = {"envd": {"sha256": "envd"}, "packages": {"sha256": "packages"}}
        responses = {
            f"cat {builder.UNIT_GUEST}": builder.SERVICE,
            f"stat {builder.LINK_GUEST}": f'Fast link dest: "{builder.UNIT_GUEST}"',
            f"stat {builder.ENVD_GUEST}": "Mode:  0755",
        }
        with patch.object(builder, "digest", side_effect=["envd", "packages"]), \
             patch.object(builder, "debugfs", side_effect=lambda _, request: responses.get(request, "")):
            with self.assertRaisesRegex(ValueError, "shadow is missing"):
                builder.verify_image(self.work / "image", self.work, profile)

    def test_untrusted_output_parent_is_rejected(self):
        # Even root-owned /tmp has writable mode; ownership alone is insufficient.
        with self.assertRaisesRegex(ValueError, "root-owned, non-writable"):
            builder.trusted_directory(self.work)


if __name__ == "__main__":
    unittest.main()
