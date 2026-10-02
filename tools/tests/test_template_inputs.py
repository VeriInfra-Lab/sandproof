import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import signal
import shutil
import sys
import tarfile
import tempfile
import time
import unittest
from unittest.mock import MagicMock, patch

from tools.template_inputs import InvalidTemplate, LayerChecks, check_members, extract_context, inspect_oci, startup_config
from tools.template_worker import AGENT, BOOT, Commands, LogBudget, check_tools, guest_path, install_platform, install_loopback_hosts, runtime_defaults, terminate, validate_request


def archive_bytes(entries):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w") as archive:
        for name, data, kind, link in entries:
            entry = tarfile.TarInfo(name)
            entry.type, entry.linkname = kind, link
            entry.size = len(data) if kind == tarfile.REGTYPE else 0
            entry.mode = 0o6755
            archive.addfile(entry, io.BytesIO(data) if entry.size else None)
    return output.getvalue()


class TemplateInputTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def context(self, entries, **limits):
        source = self.root / "context.tar"
        source.write_bytes(archive_bytes(entries))
        extract_context(source, self.root / "context", max_bytes=limits.get("max_bytes", 1000),
                        max_entries=limits.get("max_entries", 20))

    def test_context_copy_internal_link_and_modes(self):
        self.context([("Dockerfile", b"FROM scratch\nCOPY payload /payload", tarfile.REGTYPE, ""),
                      ("payload", b"hello", tarfile.REGTYPE, ""), ("link", b"", tarfile.SYMTYPE, "payload")])
        self.assertEqual((self.root / "context/link").read_bytes(), b"hello")
        self.assertEqual((self.root / "context/payload").stat().st_mode & 0o7777, 0o755)

    def test_localhost_missing_empty_custom_and_ipv6_hosts(self):
        for old, expected in (
            (None, b"127.0.0.1 localhost\n"),
            (b"", b"127.0.0.1 localhost\n"),
            (b"192.0.2.10 service # localhost", b"192.0.2.10 service # localhost\n127.0.0.1 localhost\n"),
            (b"::1 localhost\n", b"::1 localhost\n127.0.0.1 localhost\n"),
            (b"127.0.0.1 LocalHost local\n", b"127.0.0.1 LocalHost local\n"),
        ):
            with self.subTest(old=old), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "etc").mkdir()
                hosts = root / "etc/hosts"
                if old is not None:
                    hosts.write_bytes(old)
                install_loopback_hosts(root)
                self.assertEqual(hosts.read_bytes(), expected)
                self.assertEqual(hosts.stat().st_mode & 0o777, 0o644)
                install_loopback_hosts(root)
                self.assertEqual(hosts.read_bytes(), expected)

    def test_localhost_rejects_conflicting_address_without_changing_input(self):
        for content in (b"192.0.2.1 localhost\n", b"invalid localhost\n", b"127.0.0.1 localhost\n192.0.2.1 localhost\n"):
            with self.subTest(content=content), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "etc").mkdir()
                hosts = root / "etc/hosts"
                hosts.write_bytes(content)
                with self.assertRaises(InvalidTemplate):
                    install_loopback_hosts(root)
                self.assertEqual(hosts.read_bytes(), content)

    def test_localhost_replaces_links_without_writing_the_target(self):
        for link in ('symbolic', 'hard'):
            with self.subTest(link=link), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp) / 'root'
                (root / 'etc').mkdir(parents=True)
                target = Path(tmp) / 'untouched'
                target.write_bytes(b'192.0.2.10 service\n')
                hosts = root / 'etc/hosts'
                if link == 'symbolic':
                    hosts.symlink_to(target)
                else:
                    os.link(target, hosts)
                install_loopback_hosts(root)
                self.assertEqual(target.read_bytes(), b'192.0.2.10 service\n')
                self.assertNotEqual(hosts.stat().st_ino, target.stat().st_ino)
                self.assertFalse(hosts.is_symlink())
                self.assertIn(b'127.0.0.1 localhost\n', hosts.read_bytes())

    def test_localhost_bounds_and_special_files_fail_closed(self):
        for kind in ('large', 'directory', 'fifo'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / 'etc').mkdir()
                hosts = root / 'etc/hosts'
                if kind == 'large':
                    hosts.write_bytes(b'#' * 65536)
                elif kind == 'directory':
                    hosts.mkdir()
                else:
                    os.mkfifo(hosts)
                with self.assertRaises(InvalidTemplate):
                    install_loopback_hosts(root)
                self.assertTrue(hosts.exists())

    def test_context_boundaries(self):
        cases = [
            [(r"unit\x2dname", b"x", tarfile.REGTYPE, "")],
            [("link", b"", tarfile.SYMTYPE, r"unit\x2dname")],
            [("../escape", b"x", tarfile.REGTYPE, "")],
            [("/escape", b"x", tarfile.REGTYPE, "")],
            [("link", b"", tarfile.SYMTYPE, "../escape")],
            [("link", b"", tarfile.SYMTYPE, "/escape")],
            [("link", b"", tarfile.SYMTYPE, "target"), ("link/file", b"x", tarfile.REGTYPE, "")],
            [("link", b"", tarfile.LNKTYPE, "Dockerfile")],
            [("device", b"", tarfile.CHRTYPE, "")],
            [("fifo", b"", tarfile.FIFOTYPE, "")],
            [("same", b"x", tarfile.REGTYPE, ""), ("same", b"y", tarfile.REGTYPE, "")],
            [("Dockerfile", b"", tarfile.SYMTYPE, "target")],
        ]
        for case in cases:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                source = Path(temporary) / "input.tar"
                source.write_bytes(archive_bytes(case))
                with self.assertRaises((InvalidTemplate, FileExistsError)):
                    extract_context(source, Path(temporary) / "out", max_bytes=1000, max_entries=20)
                self.assertFalse((Path(temporary) / "escape").exists())

    def test_context_limits_and_no_reuse(self):
        entries = [("Dockerfile", b"FROM scratch", tarfile.REGTYPE, "")]
        with self.assertRaisesRegex(InvalidTemplate, "byte limit"):
            self.context(entries, max_bytes=1)
        with self.assertRaises(FileExistsError):
            self.context(entries)

    def make_oci(self, layer_entries=None, *, layer_bytes=None, compressed=True, diff_id=None, copies=1):
        layout = self.root / "oci"
        (layout / "blobs/sha256").mkdir(parents=True)
        def add(data, media):
            digest = hashlib.sha256(data).hexdigest()
            (layout / "blobs/sha256" / digest).write_bytes(data)
            return {"digest": "sha256:" + digest, "size": len(data), "mediaType": media}
        layer = layer_bytes if layer_bytes is not None else archive_bytes(
            layer_entries or [("etc/value", b"hello", tarfile.REGTYPE, ""),
                              ("etc/.wh.old", b"", tarfile.REGTYPE, ""),
                              ("bin", b"", tarfile.SYMTYPE, "/usr/bin")])
        layer_descriptor = add(gzip.compress(layer) if compressed else layer,
                               "application/vnd.oci.image.layer.v1.tar" + ("+gzip" if compressed else ""))
        config = {"os": "linux", "architecture": "amd64", "rootfs": {"type": "layers", "diff_ids": [
            diff_id or "sha256:" + hashlib.sha256(layer).hexdigest()] * copies},
            "config": {"Env": ["VALUE=hello"], "User": "1000"}}
        config_descriptor = add(json.dumps(config).encode(), "application/vnd.oci.image.config.v1+json")
        manifest = {"schemaVersion": 2, "config": config_descriptor, "layers": [layer_descriptor] * copies}
        descriptor = add(json.dumps(manifest).encode(), "application/vnd.oci.image.manifest.v1+json")
        (layout / "index.json").write_text(json.dumps({"schemaVersion": 2, "manifests": [descriptor]}))
        (layout / "oci-layout").write_text('{"imageLayoutVersion":"1.0.0"}')
        return layout, layer_descriptor

    def test_oci_hashes_gzip_whiteout_absolute_guest_link(self):
        layout, _ = self.make_oci()
        result = inspect_oci(layout, max_bytes=100000, max_entries=20)
        self.assertEqual(result["container_config"]["User"], "1000")
        self.assertTrue(result["image_digest"].startswith("sha256:"))

    def test_oci_layer_is_decompressed_once_for_hash_and_members(self):
        layout, _ = self.make_oci()
        # Both gzip.open and tarfile's gzip reader use this actual decoder.
        # Reopening the layer for a second scan regresses large-image latency.
        with patch('gzip.GzipFile', wraps=gzip.GzipFile) as decompressor:
            inspect_oci(layout, max_bytes=100000, max_entries=20)
        self.assertEqual(decompressor.call_count, 1)

    def test_reused_layer_checks_still_charge_every_occurrence_and_current_limits(self):
        layer = archive_bytes([('payload', b'original', tarfile.REGTYPE, '')])
        layout, _ = self.make_oci(layer_bytes=layer, copies=2)
        checks = LayerChecks(layout)
        with patch('gzip.GzipFile', wraps=gzip.GzipFile) as decompressor:
            for _ in range(2):
                inspect_oci(layout, max_bytes=2 * len(layer), max_entries=2, layer_checks=checks)
            with self.assertRaisesRegex(InvalidTemplate, 'expanded byte limit'):
                inspect_oci(layout, max_bytes=2 * len(layer) - 1, max_entries=2, layer_checks=checks)
            with self.assertRaisesRegex(InvalidTemplate, 'entry limit'):
                inspect_oci(layout, max_bytes=2 * len(layer), max_entries=1, layer_checks=checks)
        self.assertEqual(decompressor.call_count, 1)

    def test_layer_checks_cannot_be_reused_for_another_or_replaced_directory(self):
        layout, _ = self.make_oci()
        checks = LayerChecks(layout)
        inspect_oci(layout, max_bytes=100000, max_entries=20, layer_checks=checks)
        other = self.root / 'other'
        shutil.copytree(layout, other)
        with self.assertRaisesRegex(InvalidTemplate, 'another layout'):
            inspect_oci(other, max_bytes=100000, max_entries=20, layer_checks=checks)
        layout.rename(self.root / 'previous')
        other.rename(layout)
        with self.assertRaisesRegex(InvalidTemplate, 'another layout'):
            inspect_oci(layout, max_bytes=100000, max_entries=20, layer_checks=checks)

    def test_failed_gzip_validation_never_becomes_reusable(self):
        layer = archive_bytes([('payload', b'original', tarfile.REGTYPE, '')])
        damaged = gzip.compress(layer)[:-4]
        with patch('gzip.compress', return_value=damaged):
            layout, _ = self.make_oci(layer_bytes=layer)
        checks = LayerChecks(layout)
        for _ in range(2):
            with self.assertRaises(EOFError):
                inspect_oci(layout, max_bytes=100000, max_entries=20, layer_checks=checks)

    def test_oci_tar_eof_does_not_skip_trailing_bytes_or_diff_id(self):
        layer = archive_bytes([('payload', b'original', tarfile.REGTYPE, '')])
        padded = layer + b'\0' * (2 * 1024 * 1024) + b'trailing bytes'
        for compressed in (False, True):
            for bad_diff in (False, True):
                with self.subTest(compressed=compressed, bad_diff=bad_diff), tempfile.TemporaryDirectory() as tmp:
                    self.root = Path(tmp)
                    layout, _ = self.make_oci(layer_bytes=padded, compressed=compressed,
                        diff_id='sha256:' + hashlib.sha256(layer).hexdigest() if bad_diff else None)
                    if bad_diff:
                        with self.assertRaisesRegex(InvalidTemplate, 'diff_id mismatch'):
                            inspect_oci(layout, max_bytes=len(padded), max_entries=1)
                    else:
                        inspect_oci(layout, max_bytes=len(padded), max_entries=1)
                    # An uncompressed blob exceeds the descriptor bound before
                    # parsing; gzip must also charge bytes after the tar EOF.
                    message = 'expanded byte limit' if compressed else 'invalid OCI descriptor'
                    with self.assertRaisesRegex(InvalidTemplate, message):
                        inspect_oci(layout, max_bytes=len(padded) - 1, max_entries=1)

    def test_oci_gzip_footer_and_later_member_are_checked_after_tar_eof(self):
        layer = archive_bytes([('payload', b'original', tarfile.REGTYPE, '')])
        encoded = gzip.compress(layer)
        later = gzip.compress(b'\0' * 32768)
        with patch('gzip.compress', return_value=encoded + later):
            layout, _ = self.make_oci(layer_bytes=layer + b'\0' * 32768)
        inspect_oci(layout, max_bytes=len(layer) + 32768, max_entries=1)
        for damaged in (encoded[:-4], encoded[:-8] + b'\0' * 8, encoded + later[:-4]):
            with self.subTest(damaged_size=len(damaged)), tempfile.TemporaryDirectory() as tmp:
                self.root = Path(tmp)
                with patch('gzip.compress', return_value=damaged):
                    layout, _ = self.make_oci(layer_bytes=layer)
                with self.assertRaises((EOFError, gzip.BadGzipFile)):
                    inspect_oci(layout, max_bytes=100000, max_entries=1)

    def test_oci_streaming_keeps_cumulative_layer_byte_and_entry_limits(self):
        layer = archive_bytes([('payload', b'original', tarfile.REGTYPE, '')])
        layout, _ = self.make_oci(layer_bytes=layer, copies=2)
        inspect_oci(layout, max_bytes=2 * len(layer), max_entries=2)
        with self.assertRaisesRegex(InvalidTemplate, 'expanded byte limit'):
            inspect_oci(layout, max_bytes=2 * len(layer) - 1, max_entries=2)
        with self.assertRaisesRegex(InvalidTemplate, 'entry limit'):
            inspect_oci(layout, max_bytes=2 * len(layer), max_entries=1)

    def test_oci_linux_backslashes_remain_literal_in_names_and_links(self):
        unit = r"usr/lib/systemd/system/system-systemd\x2dcryptsetup.slice"
        entries = [(unit, b"synthetic unit\n", tarfile.REGTYPE, ""),
                   (r"units\hardlink", b"", tarfile.LNKTYPE, unit),
                   ("unit-link", b"", tarfile.SYMTYPE, "/" + unit),
                   (r"literal\..\name", b"literal", tarfile.REGTYPE, "")]
        layout, _ = self.make_oci(entries)
        inspect_oci(layout, max_bytes=100000, max_entries=20)
        with tarfile.open(fileobj=io.BytesIO(archive_bytes(entries))) as archive:
            checked = check_members(archive, max_bytes=100000, max_entries=20, context=False)
        self.assertEqual([(m.name, m.linkname) for m in checked], [(n, link) for n, _, _, link in entries])

    def test_oci_backslash_support_does_not_allow_posix_path_escape(self):
        for path in (r"/absolute\name", r"../outside\name", r"dir\name/../../outside", "bad\x00name", ""):
            for hardlink in (False, True):
                with self.subTest(path=path, hardlink=hardlink):
                    entry = tarfile.TarInfo("link" if hardlink else path)
                    entry.type = tarfile.LNKTYPE if hardlink else tarfile.REGTYPE
                    entry.linkname = path if hardlink else ""
                    with self.assertRaises(InvalidTemplate):
                        check_members([entry], max_bytes=100000, max_entries=20, context=False)

    def test_oci_bad_hash_bomb_path_device_and_multiindex(self):
        layout, layer = self.make_oci()
        with self.assertRaisesRegex(InvalidTemplate, "expanded byte limit"):
            inspect_oci(layout, max_bytes=1024, max_entries=20)
        path = layout / "blobs/sha256" / layer["digest"][7:]
        path.write_bytes(b"x" * layer["size"])
        with self.assertRaisesRegex(InvalidTemplate, "fingerprint"):
            inspect_oci(layout, max_bytes=100000, max_entries=20)
        for name, kind in (("../outside", tarfile.REGTYPE), ("dev/x", tarfile.CHRTYPE)):
            with tempfile.TemporaryDirectory() as temporary:
                old = self.root
                self.root = Path(temporary)
                try:
                    layout, _ = self.make_oci([(name, b"", kind, "")])
                    with self.assertRaises(InvalidTemplate):
                        inspect_oci(layout, max_bytes=100000, max_entries=20)
                    index = json.loads((layout / "index.json").read_text())
                    index["manifests"] *= 2
                    (layout / "index.json").write_text(json.dumps(index))
                    with self.assertRaisesRegex(InvalidTemplate, "one OCI image"):
                        inspect_oci(layout, max_bytes=100000, max_entries=20)
                finally:
                    self.root = old

    def test_startup_preserved_not_executed_and_rejects_unsupported(self):
        config = {"Env": ["KEY=$(touch /tmp/nope)"], "WorkingDir": "/app", "User": "1000:1000",
                  "Entrypoint": ["/entry", "a b"], "Cmd": ["$not_shell"]}
        self.assertEqual(startup_config(config), config)
        for changes in ({"Volumes": {"/data": {}}}, {"WorkingDir": "/../bad"}, {"Env": ["BAD"]},
                        {"Cmd": "not argv"}, {"User": "x\x00y"}):
            with self.subTest(changes=changes), self.assertRaises(InvalidTemplate):
                startup_config({**config, **changes})

    def test_platform_resolver_never_follows_guest_absolute_link_on_host(self):
        root = self.root / "root"
        (root / "usr/sbin").mkdir(parents=True)
        (root / "sbin").symlink_to("/usr/sbin")
        self.assertEqual(guest_path(root, "/sbin/init"), root / "usr/sbin/init")
        (root / "bad").symlink_to("../outside")
        with self.assertRaises(InvalidTemplate):
            guest_path(root, "/bad/x")
        (root / "loop").symlink_to("loop")
        with self.assertRaisesRegex(InvalidTemplate, "loop"):
            guest_path(root, "/loop")

    def test_runtime_defaults_resolve_numeric_user_and_preserve_argv(self):
        root = self.root / "root"
        (root / "etc").mkdir(parents=True)
        (root / "workspace").mkdir()
        (root / "etc/passwd").write_text("root:x:0:0:root:/root:/bin/sh\napp:x:1000:1000:app:/home/app:/bin/sh\n")
        (root / "etc/group").write_text("root:x:0:\napp:x:1000:\nother:x:1001:\n")
        config = startup_config({"Env": ["VALUE=old", "VALUE=literal $VALUE"], "User": "1000:1000",
                                 "WorkingDir": "/workspace", "Entrypoint": ["/entry", "a b"], "Cmd": ["$NO;touch bad"]})
        defaults = runtime_defaults(root, config)
        self.assertEqual(defaults, {"env_vars": {"VALUE": "literal $VALUE"}, "user": "app",
                                    "workdir": "/workspace", "argv": ["/entry", "a b", "$NO;touch bad"]})
        for change in ({"User": "missing"}, {"User": "app:other"}, {"User": "app:"},
                       {"WorkingDir": "/missing"}, {"Cmd": [""], "Entrypoint": []},
                       {"Cmd": ["x"*8192]*5}):
            with self.subTest(change=change), self.assertRaises(InvalidTemplate):
                runtime_defaults(root, {**config, **change})
        (root / "etc/passwd").write_text((root / "etc/passwd").read_text() + "alias:x:1000:1000:alias:/:/bin/sh\n")
        with self.assertRaisesRegex(InvalidTemplate, "one real account"):
            runtime_defaults(root, config)

    def test_startup_false_values_and_combined_limits_are_not_silently_defaulted(self):
        for change in ({"Cmd": False}, {"Env": 0}, {"User": False}, {"WorkingDir": []}, {"Cmd": ["汉"*3000]}):
            with self.subTest(change=change), self.assertRaises(InvalidTemplate):
                startup_config(change)

    def test_platform_reserves_trusted_init(self):
        root = self.root / "root"
        for path in ("etc", "usr/sbin", "opt"):
            (root / path).mkdir(parents=True)
        (root / "sbin").symlink_to("usr/sbin")
        (root / "etc/os-release").write_text('ID=debian\n')
        (root / "etc/passwd").write_text('root:x:0:0:root:/root:/bin/sh\n')
        binary = self.root / "binary"
        binary.write_bytes(b"trusted")
        install_platform(root, binary, binary)
        self.assertEqual((root / "usr/sbin/init").readlink(), Path("/opt/sandproof/busybox"))
        self.assertIn("/opt/sandproof/agent", (root / "etc/inittab").read_text())
        self.assertEqual((root / "etc/hosts").read_bytes(), b"127.0.0.1 localhost\n")
        with self.assertRaisesRegex(InvalidTemplate, "reserved"):
            install_platform(root, binary, binary)

    def test_boot_order_and_health_fail_closed(self):
        self.assertLess(BOOT.index("mount -t proc"), BOOT.index("mount -o remount,rw"))
        self.assertLess(BOOT.index("mount -o remount,rw"), BOOT.index("rm -f /run/sandproof-boot-ready"))
        self.assertLess(BOOT.index("rm -f /run/sandproof-boot-ready"), BOOT.index("mount -t sysfs"))
        self.assertIn("grep -q ' /dev devtmpfs ' /proc/mounts", BOOT)
        self.assertLess(BOOT.index("ip route add default"), BOOT.index("touch /run/sandproof-boot-ready"))
        self.assertLess(AGENT.index("test -f /run/sandproof-boot-ready"), AGENT.index("exec /opt/sandproof/envd"))

    def test_boot_shared_memory_mount_and_failure_gate(self):
        # Execute the actual shell control flow with a recording applet binary;
        # this test never mounts filesystems or touches host /dev or /run.
        applet = self.root / 'busybox'
        applet.write_text(f'#!{sys.executable}\n' + '''import json, os, sys
args = sys.argv[1:]
with open(os.environ['BOOT_TEST_LOG'], 'a') as out:
    out.write(json.dumps(args) + '\\n')
if args[0] == 'grep':
    sys.exit(int(os.environ['BOOT_TEST_DEV_MISSING']))
if args[0] == 'mount' and args[-1] == '/dev/shm':
    sys.exit(int(os.environ['BOOT_TEST_SHM_FAILURE']))
''')
        applet.chmod(0o755)
        boot = self.root / 'boot'
        boot.write_text(BOOT.replace('B=/opt/sandproof/busybox', f'B={applet}'))
        log = self.root / 'calls.jsonl'
        for dev_missing in (0, 1):
            for failure in (0, 73):
                with self.subTest(dev_missing=dev_missing, failure=failure):
                    log.write_text('')
                    result = subprocess.run(['/bin/sh', str(boot)], capture_output=True, timeout=10,
                        env={**os.environ, 'BOOT_TEST_LOG': str(log),
                             'BOOT_TEST_DEV_MISSING': str(dev_missing),
                             'BOOT_TEST_SHM_FAILURE': str(failure)})
                    calls = [json.loads(line) for line in log.read_text().splitlines()]
                    self.assertEqual(result.returncode, failure, result.stderr)
                    mounts = [call for call in calls if call[0] == 'mount' and call[-1] == '/dev/shm']
                    self.assertEqual(len(mounts), 1)
                    mount = mounts[0]
                    self.assertEqual(mount[1:4], ['-t', 'tmpfs', '-o'])
                    self.assertEqual(set(mount[4].split(',')),
                                     {'nosuid', 'nodev', 'noexec', 'mode=1777', 'size=64m'})
                    mkdir = next(call for call in calls if call[0] == 'mkdir' and '/dev/shm' in call)
                    self.assertLess(calls.index(mkdir), calls.index(mount))
                    if dev_missing:
                        device_mount = ['mount', '-t', 'devtmpfs', 'devtmpfs', '/dev']
                        self.assertLess(calls.index(device_mount), calls.index(mkdir))
                    ready = ['touch', '/run/sandproof-boot-ready']
                    if failure:
                        self.assertNotIn(ready, calls)
                        self.assertEqual(calls[-1], mount)
                    else:
                        self.assertEqual(calls[-1], ready)

    def test_worker_limits_are_explicit_no_boolean_or_unknown_field(self):
        request = {"kind": "dockerfile", "disk_mib": 1024, "min_free_mib": 64, "max_input_bytes": 100,
                   "max_expanded_bytes": 10000 * 1024, "max_entries": 100, "timeout_seconds": 30, "max_log_bytes": 1024}
        validate_request(request)
        for changes in ({"timeout_seconds": True}, {"disk_mib": 0}, {"min_free_mib": 1024}, {"extra": 1}):
            with self.subTest(changes=changes), self.assertRaises(InvalidTemplate):
                validate_request({**request, **changes})

    def test_busybox_requires_cleanup_and_filesystem_stat_capabilities(self):
        binary = self.root / 'tool'
        binary.write_bytes(b'trusted fixture')
        descriptor = {'path': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
        profile = dict.fromkeys(('buildctl', 'buildkitd', 'buildkit-runc', 'umoci', 'envd', 'busybox'), descriptor)
        applets = {'sh', 'init', 'mount', 'grep', 'mkdir', 'chmod', 'ln', 'ip', 'touch', 'reboot', 'umount', 'rm', 'stat'}
        listing = '\n'.join(sorted(applets)).encode()
        for missing in ('rm', 'stat'):
            with self.subTest(missing=missing), patch('tools.template_worker.subprocess.check_output',
                    return_value='\n'.join(sorted(applets - {missing})).encode()) as command:
                with self.assertRaisesRegex(InvalidTemplate, 'missing required'):
                    check_tools(profile)
                self.assertEqual(command.call_count, 1)
        for output in (b'ef53\n', b'1021994\n'):
            with self.subTest(output=output), patch('tools.template_worker.subprocess.check_output',
                    side_effect=[listing, output]) as command:
                self.assertEqual(check_tools(profile)['busybox'], binary)
                self.assertEqual(command.call_args.args[0], [str(binary), 'stat', '-f', '-c', '%t', '/'])
        for invalid in (b'', b'%t\n', b'wrong format\n', subprocess.CalledProcessError(1, 'stat', output=b'1021994\n')):
            with self.subTest(invalid=invalid), patch('tools.template_worker.subprocess.check_output',
                    side_effect=[listing, invalid]), self.assertRaises((InvalidTemplate, subprocess.CalledProcessError)):
                check_tools(profile)

    def test_fresh_run_mount_restores_lock_symlink_target_before_guest_is_ready(self):
        # Run the real boot shell against a sandboxed filesystem applet. Its
        # simulated /run mount discards baked-in content; no host mounts occur.
        applet = self.root / 'lock-busybox'
        applet.write_text(f'#!{sys.executable}\n' + '''import os, shutil, sys
from pathlib import Path
root = Path(os.environ['BOOT_TEST_ROOT'])
args = sys.argv[1:]
if args[0] == 'mount' and args[-1] == '/run':
    shutil.rmtree(root / 'run')
    (root / 'run').mkdir()
elif args[0] == 'mkdir':
    if '/run/lock' in args and os.environ['BOOT_TEST_FAILURE'] == 'mkdir':
        sys.exit(73)
    for value in args[1:]:
        if value.startswith('/'):
            (root / value.lstrip('/')).mkdir(parents=True, exist_ok=True)
elif args[0] == 'chmod':
    if args[-1] == '/run/lock' and os.environ['BOOT_TEST_FAILURE'] == 'chmod':
        sys.exit(74)
    (root / args[-1].lstrip('/')).chmod(int(args[1], 8))
elif args[0] == 'touch':
    (root / args[-1].lstrip('/')).touch()
''')
        applet.chmod(0o755)
        boot = self.root / 'lock-boot'
        boot.write_text(BOOT.replace('B=/opt/sandproof/busybox', f'B={applet}'))
        for failure, code in (('', 0), ('mkdir', 73), ('chmod', 74)):
            with self.subTest(failure=failure):
                root = self.root / ('guest-' + (failure or 'success'))
                (root / 'run/lock').mkdir(parents=True)
                (root / 'run/lock/baked-in').touch()
                (root / 'var').mkdir()
                (root / 'var/lock').symlink_to('../run/lock')
                result = subprocess.run(['/bin/sh', str(boot)], capture_output=True, timeout=10,
                    env={**os.environ, 'BOOT_TEST_ROOT': str(root), 'BOOT_TEST_FAILURE': failure})
                self.assertEqual(result.returncode, code, result.stderr)
                self.assertFalse((root / 'run/lock/baked-in').exists())
                self.assertEqual((root / 'run/sandproof-boot-ready').exists(), not failure)
                self.assertEqual((root / 'var/lock').readlink(), Path('../run/lock'))
                if not failure:
                    # Reproduce the original application's operation through
                    # its unchanged /var/lock symlink after boot.
                    (root / 'var/lock/mailman').mkdir()
                    self.assertTrue((root / 'run/lock/mailman').is_dir())
                    self.assertEqual((root / 'run/lock').stat().st_mode & 0o7777, 0o1777)

    def test_command_nonzero_deadline_and_bounded_output(self):
        log = io.BytesIO()
        runner = Commands(time.monotonic() + 10, log, 3)
        # These commands exit by themselves. Avoid testing Linux process-group
        # signal permissions through the macOS execution sandbox.
        with patch("tools.template_worker.terminate", side_effect=lambda child: child.wait(timeout=2)):
            with self.assertRaisesRegex(InvalidTemplate, "log limit"):
                runner.run(["/bin/sh", "-c", "printf abcdef"])
        self.assertEqual(log.getvalue(), b"abc")
        with patch("tools.template_worker.terminate", side_effect=lambda child: child.wait(timeout=2)):
            with self.assertRaises(RuntimeError):
                Commands(time.monotonic() + 10, io.BytesIO(), 10).run(["/bin/sh", "-c", "exit 7"])
        with self.assertRaises(TimeoutError):
            Commands(time.monotonic() - 1, io.BytesIO(), 10).run(["/bin/sh", "-c", "exit 0"])

    def test_process_group_cleanup_and_shared_log_budget(self):
        child = MagicMock(pid=12345)
        child.poll.return_value = None
        child.wait.side_effect = [subprocess.TimeoutExpired("tool", 3), 0]
        with patch("tools.template_worker.os.killpg") as kill:
            terminate(child)
            self.assertEqual(kill.call_args_list[0].args, (12345, signal.SIGTERM))
            self.assertEqual(kill.call_args_list[1].args, (12345, signal.SIGKILL))
        budget = LogBudget(10)
        self.assertEqual(budget.take(6), 6)
        self.assertEqual(budget.take(6), 4)
        self.assertEqual(budget.take(1), 0)


class RuntimeReadinessTests(unittest.TestCase):
    """Generated shell control flow, with no real mounts, PID 1 or guest execution."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name)
        self.root = self.work / 'root'
        for name in ('etc', 'sbin', 'run'):
            (self.root / name).mkdir(parents=True)
        (self.root / 'etc/os-release').write_text('ID=debian\n')
        (self.root / 'etc/passwd').write_text('root:x:0:0:root:/root:/bin/sh\n')
        self.target = self.work / 'untouched'
        self.target.write_text('image data')
        self.marker = self.root / 'run/sandproof-boot-ready'
        self.busybox = self.work / 'fake-busybox'
        self.busybox.write_text(f'#!{sys.executable}\n' + r'''import json, os, sys
from pathlib import Path
root = Path(os.environ['BOOT_TEST_ROOT'])
work = root.parent
args = sys.argv[1:]
guest_args = [value.replace(str(root), '') for value in args]
with (work / 'calls.jsonl').open('a') as output:
    output.write(json.dumps(guest_args) + '\n')
if ' '.join(guest_args) == os.environ.get('BOOT_TEST_FAILURE'):
    if args[0] == 'stat':
        print('1021994')  # Correct-looking stdout must not hide a failed syscall.
    sys.exit(73)
if args[0] == 'grep':
    sys.exit(0)
elif args[0] == 'rm':
    assert guest_args == ['rm', '-f', '/run/sandproof-boot-ready']
    (root / 'run/sandproof-boot-ready').unlink(missing_ok=True)
elif args[0] == 'mount':
    if guest_args[-1] == '/run':
        # Model a fresh overlay with private directories, never mount anything.
        (root / 'run').rename(root / 'run-underlay')
        (root / 'run').mkdir()
        (work / 'run-is-tmpfs').touch()
elif args[0] == 'stat':
    assert guest_args == ['stat', '-f', '-c', '%t', '/run']
    print('1021994' if (work / 'run-is-tmpfs').exists() else 'ef53')
elif args[0] == 'touch':
    assert guest_args == ['touch', '/run/sandproof-boot-ready']
    (root / 'run/sandproof-boot-ready').touch()
elif args[0] not in ('mkdir', 'chmod', 'ln', 'ip'):
    raise AssertionError('unexpected applet: ' + repr(args))
''')
        self.busybox.chmod(0o755)
        self.envd = self.work / 'fake-envd'
        self.envd.write_text('#!/bin/sh\nprintf "envd_started\\n"\n')
        self.envd.chmod(0o755)
        self.log = self.work / 'calls.jsonl'
        self.log.write_text('')

    def seed_marker(self, kind):
        if kind == 'regular':
            self.marker.write_text('imported ready')
        elif kind == 'symlink':
            self.marker.symlink_to(self.target)
        elif kind == 'dangling':
            self.marker.symlink_to(self.work / 'missing')
        elif kind == 'directory':
            self.marker.mkdir()
        elif kind == 'fifo':
            os.mkfifo(self.marker)
        elif kind == 'hardlink':
            os.link(self.target, self.marker)

    def install(self):
        install_platform(self.root, self.envd, self.busybox)
        # Only trusted executable paths and the shell's /run predicates are
        # relocated. All mount/network/etc. operations go to recording fakes.
        for name in ('boot', 'agent'):
            path = self.root / 'opt/sandproof' / name
            path.write_text(path.read_text().replace('/opt/sandproof', str(self.root / 'opt/sandproof'))
                            .replace('/run', str(self.root / 'run')))

    def run_script(self, name, failure=''):
        return subprocess.run(['/bin/sh', str(self.root / 'opt/sandproof' / name)],
            env={**os.environ, 'BOOT_TEST_ROOT': str(self.root), 'BOOT_TEST_FAILURE': failure},
            capture_output=True, text=True, timeout=10)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def assert_agent_refuses(self, failure=''):
        agent = self.run_script('agent', failure)
        self.assertNotEqual(agent.returncode, 0)
        self.assertNotIn('envd_started', agent.stdout)

    def test_install_discards_imported_ready_leaf_without_following_links(self):
        for kind in ('regular', 'symlink', 'dangling', 'fifo', 'hardlink'):
            with self.subTest(kind=kind):
                case = RuntimeReadinessTests()
                case.setUp()
                try:
                    case.seed_marker(kind)
                    case.install()
                    self.assertFalse(case.marker.exists())
                    self.assertFalse(case.marker.is_symlink())
                    self.assertEqual(case.target.read_text(), 'image data')
                    case.assert_agent_refuses()
                finally:
                    case.doCleanups()

    def test_install_rejects_directory_or_failed_invalidation(self):
        self.seed_marker('directory')
        with self.assertRaises((InvalidTemplate, OSError)):
            self.install()
        self.marker.rmdir()
        shutil.rmtree(self.root / 'opt/sandproof')
        self.seed_marker('regular')
        unlink = Path.unlink
        def deny_marker(path, *args, **kwargs):
            if path == self.marker:
                raise PermissionError('injected readiness invalidation failure')
            return unlink(path, *args, **kwargs)
        with patch.object(Path, 'unlink', deny_marker), self.assertRaises(PermissionError):
            self.install()
        self.assertEqual(self.marker.read_text(), 'imported ready')

    def test_install_rejects_redirected_run_before_removing_any_target(self):
        self.root.joinpath('run').rmdir()
        outside = self.work / 'outside'
        outside.mkdir()
        marker = outside / 'sandproof-boot-ready'
        marker.write_text('do not remove')
        self.root.joinpath('run').symlink_to(outside)
        with self.assertRaisesRegex(InvalidTemplate, 'mount path redirected'):
            self.install()
        self.assertEqual(marker.read_text(), 'do not remove')

    def test_clean_boot_admits_envd_and_respawn_after_all_setup(self):
        self.install()
        self.assert_agent_refuses()
        boot = self.run_script('boot')
        self.assertEqual(boot.returncode, 0, boot.stderr)
        self.assertEqual(self.calls()[-1], ['touch', '/run/sandproof-boot-ready'])
        for _ in range(2):
            agent = self.run_script('agent')
            self.assertEqual(agent.returncode, 0, agent.stderr)
            self.assertEqual(agent.stdout, 'envd_started\n')
        self.assertEqual(sum(call[0] == 'touch' for call in self.calls()), 1)

    def test_each_boot_failure_blocks_envd_even_with_stale_input(self):
        failures = (
            'mount -t proc proc /proc',
            'mount -o remount,rw /',
            'rm -f /run/sandproof-boot-ready',
            'mount -t sysfs sysfs /sys',
            'mount -t tmpfs -o nosuid,nodev,noexec,mode=1777,size=64m tmpfs /dev/shm',
            'mount -t tmpfs -o mode=755 tmpfs /run',
            'ip route add default via 172.16.0.1 dev eth0',
            'touch /run/sandproof-boot-ready',
        )
        for kind in ('regular', 'symlink'):
            for failure in failures:
                with self.subTest(kind=kind, failure=failure):
                    # Every case has its own installed root and fake mount state.
                    case = RuntimeReadinessTests()
                    case.setUp()
                    try:
                        case.seed_marker(kind)
                        case.install()
                        # Also exercise defense in depth against an on-disk
                        # stale leaf if cleanup had not removed it. This is
                        # an ext4 cold-boot model, not active guest-root defense.
                        if not case.marker.exists() and not case.marker.is_symlink():
                            case.seed_marker(kind)
                        boot = case.run_script('boot', failure)
                        self.assertEqual(boot.returncode, 73, boot.stderr)
                        calls = case.calls()
                        self.assertEqual(' '.join(calls[-1]), failure)
                        if failure != failures[-1]:
                            self.assertNotIn(['touch', '/run/sandproof-boot-ready'], calls)
                        case.assert_agent_refuses()
                        self.assertEqual(case.target.read_text(), 'image data')
                    finally:
                        case.doCleanups()
    def test_agent_rejects_symlink_marker_or_failed_filesystem_check(self):
        self.install()
        boot = self.run_script('boot')
        self.assertEqual(boot.returncode, 0, boot.stderr)
        self.assert_agent_refuses('stat -f -c %t /run')
        self.marker.unlink()
        self.seed_marker('symlink')
        self.assert_agent_refuses()
        self.assertEqual(self.target.read_text(), 'image data')
        self.marker.unlink()
        self.seed_marker('regular')
        self.root.joinpath('run').rename(self.root / 'old-run')
        self.root.joinpath('run').symlink_to(self.root / 'old-run')
        self.assert_agent_refuses()


if __name__ == "__main__":
    unittest.main()
