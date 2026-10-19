"""Join in-progress native-to-Python notifications before closing a fixture loop."""
import asyncio
import threading


class JoinedCallbackLoop(asyncio.SelectorEventLoop):
    def __init__(self):
        self._callback_lock = threading.RLock()
        super().__init__()

    def call_soon_threadsafe(self, callback, *args, context=None):
        # A native SDK thread may have enqueued its result while still inside
        # the socket wakeup (which releases the GIL). Receiving that result is
        # not evidence that this cross-thread Python call has returned.
        with self._callback_lock:
            return super().call_soon_threadsafe(callback, *args, context=context)

    def close(self):
        # Keep Python alive until calls already delivering SDK results return.
        # This joins notifications, not arbitrary SDK tasks or remote effects.
        with self._callback_lock:
            super().close()
