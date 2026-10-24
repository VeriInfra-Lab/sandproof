"""Keep original refusal errors and current request-settlement checks together."""
import threading
import unittest
from unittest.mock import AsyncMock, patch

from connectrpc.code import Code
from connectrpc.errors import ConnectError

import guest_header_rejection_sdk as probe
from sdk_rejection_body import gated_sync


class RefusalObservationTests(unittest.IsolatedAsyncioTestCase):
    async def refusal(self, error):
        events, settled = [], threading.Event()
        finish = AsyncMock()
        control = object()  # Closing after the first offer never uses the gate.
        stream = gated_sync(lambda: iter([b'original message']), control, events,
                            True, settled=settled)

        class Refusal:
            def __next__(self):
                next(stream)
                raise error

            def close(self):
                stream.close()

        class Client:
            def start(self, *_args, **_kwargs):
                return Refusal()

        with patch.multiple(probe,
                settings={'partial': True, 'asynchronous': False,
                          'url': 'http://unused.invalid'},
                events=events, control=control, sync_upload_settled=settled,
                finish_gate=finish), patch.object(probe, 'sync_client', return_value=Client()):
            try:
                await probe.main()
            finally:
                self.assertTrue(settled.is_set())
                self.assertEqual(events, ['message'])
                finish.assert_not_called()

    async def test_wrong_error_is_not_hidden_by_progress_observation(self):
        with self.assertRaisesRegex(AssertionError, 'wrong original error') as failed:
            await self.refusal(ConnectError(Code.INTERNAL, 'wrong original error'))
        self.assertIsInstance(failed.exception.__cause__, AssertionError)
        self.assertEqual(str(failed.exception.__cause__), "['message']")

    async def test_correct_error_does_not_bypass_missing_tail_observation(self):
        with self.assertRaisesRegex(AssertionError, r"^\['message'\]$"):
            await self.refusal(ConnectError(Code.INVALID_ARGUMENT,
                'keepalive interval must be between 1 and 9223372036 seconds'))
