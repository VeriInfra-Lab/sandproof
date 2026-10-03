"""Output EOF must not remove the deadline from a still-running build tool."""
import io
import os
import signal
import subprocess
import sys
import threading
import unittest
from unittest.mock import patch

from tools.template_worker import Commands


class CommandDeadlineTests(unittest.TestCase):
    def closed_output(self, after_eof, *, hanging=True, after_wait=None):
        now = [100.0]
        log = io.BytesIO()
        runner = Commands(101.0, log, 4096)
        eof_seen, finished = threading.Event(), threading.Event()
        children, errors = [], []
        real_popen, real_drain = subprocess.Popen, runner.drain

        def spawn(*args, **kwargs):
            child = real_popen(*args, **kwargs)
            children.append(child)
            if after_wait is not None:
                real_wait = child.wait

                def wait(*args, **kwargs):
                    result = real_wait(*args, **kwargs)
                    if eof_seen.is_set():
                        now[0] = after_wait
                    return result

                child.wait = wait
            return child

        def drain(child):
            real_drain(child)
            # The real child has closed both output descriptors. Advance the
            # deadline clock only after that barrier, without scheduler sleeps.
            self.assertEqual(log.getvalue(), b'ready\n')
            if hanging:
                self.assertIsNone(child.poll())
            else:
                self.assertEqual(child.wait(timeout=5), 0)
            now[0] = after_eof
            eof_seen.set()

        def run():
            try:
                runner.run([sys.executable, '-c',
                            "import os, signal; os.write(1, b'ready\\n'); "
                            "os.close(1); os.close(2); " +
                            ('signal.pause()' if hanging else 'pass')])
            except BaseException as error:
                errors.append(error)
            finally:
                finished.set()

        with patch('tools.template_worker.time.monotonic', side_effect=lambda: now[0]), \
             patch('tools.template_worker.subprocess.Popen', side_effect=spawn), \
             patch.object(runner, 'drain', side_effect=drain):
            thread = threading.Thread(target=run)
            thread.start()
            try:
                self.assertTrue(eof_seen.wait(5), 'child never reached output EOF')
                self.assertTrue(finished.wait(2), 'output EOF bypassed the build deadline')
            finally:
                # The regression must reclaim its exact real child even when
                # run against the old unbounded wait implementation.
                for child in children:
                    if child.poll() is None:
                        os.killpg(child.pid, signal.SIGKILL)
                thread.join(timeout=5)
            self.assertFalse(thread.is_alive())
        self.assertEqual(len(children), 1)
        self.assertIsNotNone(children[0].poll())
        self.assertTrue(children[0].stdout.closed)
        return errors

    def test_still_running_child_cannot_wait_past_remaining_deadline_after_eof(self):
        errors = self.closed_output(100.9)
        self.assertEqual(len(errors), 1)
        self.assertIsInstance(errors[0], TimeoutError)
        self.assertEqual(str(errors[0]), 'build deadline')

    def test_expired_deadline_after_eof_reclaims_still_running_child(self):
        errors = self.closed_output(101.0)
        self.assertEqual(len(errors), 1)
        self.assertIsInstance(errors[0], TimeoutError)

    def test_successful_exit_observed_after_deadline_is_not_accepted(self):
        errors = self.closed_output(101.0, hanging=False)
        self.assertEqual(len(errors), 1)
        self.assertIsInstance(errors[0], TimeoutError)

    def test_successful_exit_before_deadline_is_accepted(self):
        self.assertEqual(self.closed_output(100.5, hanging=False), [])

    def test_deadline_crossed_while_waiting_does_not_accept_success(self):
        errors = self.closed_output(100.5, hanging=False, after_wait=101.0)
        self.assertEqual(len(errors), 1)
        self.assertIsInstance(errors[0], TimeoutError)


if __name__ == '__main__':
    unittest.main()
