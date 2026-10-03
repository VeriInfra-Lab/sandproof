import io
import threading
import unittest

from tools.template_inputs import PrefetchReader


class PrefetchTests(unittest.TestCase):
    def tearDown(self):
        self.assertFalse([thread for thread in threading.enumerate()
                          if thread.name == "oci-decode-prefetch"])

    def test_consumer_failure_with_full_queue_preserves_error_and_joins(self):
        produced = threading.Event()

        class Source:
            calls = 0

            def read(self, size):
                self.calls += 1
                if self.calls == 3:
                    produced.set()
                return b"x" * size

        source = Source()
        failure = ValueError("consumer rejected archive")
        with self.assertRaises(ValueError) as caught:
            with PrefetchReader(source, 8 * 1024**2):
                self.assertTrue(produced.wait(2), "producer did not reach queue pressure")
                raise failure
        self.assertIs(caught.exception, failure)
        self.assertEqual(source.calls, 3)

    def test_decoder_failure_propagates_and_joins(self):
        failure = OSError("decoder read failed")

        class Source:
            def read(self, size):
                raise failure

        with PrefetchReader(Source(), 1024) as reader:
            with self.assertRaises(OSError) as caught:
                reader.read(1)
        self.assertIs(caught.exception, failure)

    def test_consumption_never_reads_past_byte_limit(self):
        source = io.BytesIO(b"a" * 4096)
        with PrefetchReader(source, 1025) as reader:
            self.assertEqual(reader.read(0), b"")
            self.assertEqual(reader.read(17), b"a" * 17)
            self.assertEqual(reader.read(2000), b"a" * 1008)
            self.assertEqual(reader.read(1), b"")
        self.assertEqual(source.tell(), 1025)

    def test_early_eof_preserves_short_reads(self):
        source = io.BytesIO(b"short")
        with PrefetchReader(source, 4096) as reader:
            self.assertEqual(reader.read(2), b"sh")
            self.assertEqual(reader.read(2000), b"ort")
            self.assertEqual(reader.read(1), b"")
            self.assertEqual(reader.read(1), b"")


if __name__ == "__main__":
    unittest.main()
