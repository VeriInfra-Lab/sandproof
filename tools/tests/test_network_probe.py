"""Loopback-only compiled probe tests, not KVM/network-policy acceptance."""
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading
import unittest


class NetworkProbeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        compiler = next((path for name in ("cc", "gcc", "clang")
                         if (path := shutil.which(name))), None)
        if compiler is None:
            raise unittest.SkipTest("network probe component requires a C compiler")
        cls.directory = tempfile.TemporaryDirectory(prefix="sandproof-network-probe-")
        cls.addClassCleanup(cls.directory.cleanup)
        cls.binary = Path(cls.directory.name) / "network-probe"
        source = Path(__file__).resolve().parents[2] / "tests/system/network_probe.c"
        subprocess.run([compiler, "-std=c11", "-Wall", "-Wextra", "-Werror",
                        "-O2", str(source), "-o", str(cls.binary)],
                       check=True, capture_output=True, timeout=30)

    def probe(self, *arguments):
        return subprocess.run([str(self.binary), *map(str, arguments)],
                              capture_output=True, text=True, timeout=12)

    def test_listening_tcp_endpoint_is_allowed(self):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(1)
            result = self.probe("tcp", "127.0.0.1", listener.getsockname()[1])
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (0, "allowed\n", ""))
            connection, peer = listener.accept()
            with connection:
                self.assertEqual(peer[0], "127.0.0.1")

    def test_bound_non_listening_tcp_endpoint_is_denied(self):
        # Hold the address throughout the probe, avoiding a port-allocation race.
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as closed:
            closed.bind(("127.0.0.1", 0))
            result = self.probe("tcp", "127.0.0.1", closed.getsockname()[1])
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (0, "denied\n", ""))

    def test_malformed_input_is_not_a_denial(self):
        cases = [((), "arguments"),
                 (("udp", "127.0.0.1", "53"), "arguments"),
                 (("tcp", "localhost", "443"), "address"),
                 (("dns", "::1", "53"), "address"),
                 (("tcp", "127.0.0.999", "443"), "address")]
        for value in ("", "0", "65536", "-1", "+443", " 443", "443x",
                      "999999999999999999999999999999999"):
            cases.append((("tcp", "127.0.0.1", value), "port"))
        for arguments, stage in cases:
            with self.subTest(arguments=arguments):
                result = self.probe(*arguments)
                self.assertEqual((result.returncode, result.stdout, result.stderr),
                                 (2, "", "probe-error:" + stage + "\n"))

    def dns_reply(self, transform):
        errors = []
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server:
            server.bind(("127.0.0.1", 0))
            server.settimeout(2)

            def respond():
                try:
                    packet, peer = server.recvfrom(4096)
                    server.sendto(transform(bytearray(packet)), peer)
                except Exception as error:
                    errors.append(error)

            thread = threading.Thread(target=respond)
            thread.start()
            try:
                result = self.probe("dns", "127.0.0.1", server.getsockname()[1])
            finally:
                thread.join(3)
            self.assertFalse(thread.is_alive(), "DNS fixture thread did not stop")
            self.assertEqual(errors, [])
            return result

    def test_dns_negative_answer_is_reachable_not_denied(self):
        def nxdomain(packet):
            packet[2] |= 0x80
            packet[3] = 3
            return packet

        result = self.dns_reply(nxdomain)
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (0, "allowed\n", ""))

    def test_invalid_dns_reply_is_probe_failure_not_denied(self):
        result = self.dns_reply(lambda _packet: b"not-a-dns-response")
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (2, "", "probe-error:dns-response\n"))


if __name__ == "__main__":
    unittest.main()
