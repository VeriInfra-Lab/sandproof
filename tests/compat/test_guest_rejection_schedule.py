"""The observer must not confuse chunk handoff with another consumer poll."""
import unittest
from unittest.mock import patch

from connectrpc.code import Code
from connectrpc.errors import ConnectError

import guest_header_rejection_sdk as probe

PAYLOAD = b'original encoded message'


class Control:
    def __init__(self):
        self.sent = []

    def sendall(self, value):
        self.sent.append(value)

    def recv(self, count):
        assert count == 1
        return b'x'


def encoded(*_args):
    yield PAYLOAD


async def encoded_async(*_args):
    yield PAYLOAD


class RejectionScheduleTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.events = []
        self.control = Control()
        override = patch.multiple(probe,
            settings={'partial': True, 'asynchronous': False, 'url': 'http://unused.invalid'},
            events=self.events, control=self.control,
            original_sync=encoded, original_async=encoded_async)
        override.start()
        self.addCleanup(override.stop)

    def test_sync_tail_handoff_does_not_require_an_eof_poll(self):
        stream = probe.gated_sync()
        first, tail = next(stream), next(stream)
        self.assertEqual(first + tail, PAYLOAD)
        self.assertEqual(self.control.sent, [b'm'])
        self.assertEqual(self.events, ['message', 'waiting', 'released', 'tail_offered'])
        stream.close()

    async def test_async_tail_handoff_does_not_require_an_eof_poll(self):
        stream = probe.gated_async()
        first, tail = await anext(stream), await anext(stream)
        self.assertEqual(first + tail, PAYLOAD)
        self.assertEqual(self.control.sent, [b'm'])
        self.assertEqual(self.events, ['message', 'waiting', 'released', 'tail_offered'])
        await stream.aclose()

    def test_sync_stopping_before_tail_does_not_invent_handoff(self):
        stream = probe.gated_sync()
        self.assertEqual(next(stream), PAYLOAD[:1])
        stream.close()
        self.assertEqual(self.events, ['message'])
        self.assertEqual(self.control.sent, [])

    async def test_async_stopping_before_tail_does_not_invent_handoff(self):
        stream = probe.gated_async()
        self.assertEqual(await anext(stream), PAYLOAD[:1])
        await stream.aclose()
        self.assertEqual(self.events, ['message'])
        self.assertEqual(self.control.sent, [])

    async def refusal(self, error):
        stream = probe.gated_sync()

        class Refusal:
            def __next__(self):
                next(stream)
                raise error

            def close(self):
                stream.close()

        class Client:
            def start(self, *_args, **_kwargs):
                return Refusal()

        with patch.object(probe, 'sync_client', return_value=Client()):
            await probe.main()

    async def test_wrong_error_is_not_hidden_by_progress_observation(self):
        with self.assertRaisesRegex(AssertionError, 'wrong original error'):
            await self.refusal(ConnectError(Code.INTERNAL, 'wrong original error'))

    async def test_correct_error_does_not_bypass_missing_tail_observation(self):
        with self.assertRaisesRegex(AssertionError, r"^\['message'\]$"):
            await self.refusal(ConnectError(Code.INVALID_ARGUMENT,
                'keepalive interval must be between 1 and 9223372036 seconds'))
