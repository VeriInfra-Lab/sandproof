import ast
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(sys.platform == "linux" and shutil.which("ldd"), "Linux fixture requires ldd")
class TemplateFixtureTests(unittest.TestCase):
    def test_private_operator_umask_preserves_guest_user_access(self):
        # Read the real generator without importing the optional E2B SDK.
        module = ast.parse((ROOT / "tests/system/template_build.py").read_text())
        fixture = next(
            ast.literal_eval(node.value)
            for node in module.body
            if isinstance(node, ast.Assign)
            and any(isinstance(target, ast.Name) and target.id == "FIXTURE" for target in node.targets)
        )
        for mask in (0o022, 0o077):
            with self.subTest(umask=oct(mask)), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "tools").mkdir()
                # The generator copies BusyBox but never executes it.
                (root / "tools/busybox").write_bytes(b"synthetic busybox input\n")
                subprocess.run(
                    [sys.executable, "-c", fixture.replace("/input", str(root))],
                    check=True, umask=mask, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                )
                self.assertEqual(root.stat().st_mode & 0o777, 0o700)
                with tarfile.open(root / "context.tar") as archive:
                    # Docker COPY retains these modes; root-owned files must
                    # remain readable by the fixture's USER 1000:1000.
                    for name in ("payload", "rootfs/etc/passwd", "rootfs/etc/group",
                                 "rootfs/etc/os-release", "rootfs/workspace/start.sh"):
                        member = archive.getmember(name)
                        self.assertTrue(member.isfile(), name)
                        self.assertEqual(member.mode, 0o644, name)
                    for name in ("rootfs", "rootfs/bin", "rootfs/etc", "rootfs/workspace"):
                        member = archive.getmember(name)
                        self.assertTrue(member.isdir(), name)
                        self.assertEqual(member.mode, 0o755, name)
                    for name in ("rootfs/bin/busybox", "rootfs/bin/bash"):
                        self.assertEqual(archive.getmember(name).mode, 0o755, name)
                    self.assertEqual(archive.extractfile("payload").read(), b"copied-standard-context")


if __name__ == "__main__":
    unittest.main()
