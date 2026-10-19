"""A delivered result may precede the native caller's socket-wakeup return."""
import asyncio
import concurrent.futures
import contextvars
import threading
import unittest

from sdk_event_loop import JoinedCallbackLoop


class JoinedCallbackLoopTests(unittest.TestCase):
    def test_close_waits_for_an_already_delivered_cross_thread_wakeup(self):
        loop = JoinedCallbackLoop()
        self.addCleanup(loop.close)
        armed, attempting_close = threading.Event(), threading.Event()
        lock = threading.RLock()

        class ObservedLock:
            def __enter__(self):
                if armed.is_set():
                    attempting_close.set()
                lock.acquire()

            def __exit__(self, *args):
                lock.release()

        # Observe the close attempt at a real lock boundary, without sleeps.
        loop._callback_lock = ObservedLock()
        release, wakeup_entered = threading.Event(), threading.Event()
        original_wakeup = loop._write_to_self
        observations = []

        def delayed_wakeup():
            original_wakeup()
            wakeup_entered.set()
            if not release.wait(5):
                raise AssertionError('test did not release native wakeup')
            observations.append('wakeup-returned')

        loop._write_to_self = delayed_wakeup
        result = loop.create_future()

        def close():
            loop.close()
            observations.append('closed')

        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as workers:
            caller = workers.submit(loop.call_soon_threadsafe, result.set_result, 'delivered')
            try:
                self.assertTrue(wakeup_entered.wait(5))
                self.assertEqual(loop.run_until_complete(result), 'delivered')
                armed.set()
                closer = workers.submit(close)
                self.assertTrue(attempting_close.wait(5))
                self.assertFalse(loop.is_closed())
                release.set()
                caller.result(timeout=5)
                closer.result(timeout=5)
                self.assertEqual(observations, ['wakeup-returned', 'closed'])
            finally:
                release.set()

    def test_callback_context_cancellation_and_closed_loop_rejection_are_preserved(self):
        loop = JoinedCallbackLoop()
        self.addCleanup(loop.close)
        marker = contextvars.ContextVar('marker', default='outside')
        context = contextvars.copy_context()
        context.run(marker.set, 'inside')
        values = []
        loop.call_soon_threadsafe(lambda value: values.append((value, marker.get())),
                                  'original', context=context)
        handle = loop.call_soon_threadsafe(values.append, 'cancelled')
        handle.cancel()
        loop.run_until_complete(asyncio.sleep(0))
        self.assertEqual(values, [('original', 'inside')])
        loop.close()
        with self.assertRaises(RuntimeError):
            loop.call_soon_threadsafe(values.append, 'after-close')


if __name__ == '__main__':
    unittest.main()
