"""A request-body consumer may stop immediately after accepting the last chunk."""
import unittest
from unittest.mock import Mock
from sdk_rejection_body import gated_async, gated_sync


WIRE = b"\x00\x00\x00\x00\x07payload"


def chunks(*args):
    assert args == ('original-request',)
    yield WIRE


async def async_chunks(*args):
    for chunk in chunks(*args):
        yield chunk


def control():
    return Mock(recv=Mock(return_value=b'x'))


class RejectionBodyTests(unittest.TestCase):
    def test_sync_consumer_can_close_after_taking_the_tail_without_another_poll(self):
        events, socket = [], control()
        body = gated_sync(chunks, socket, events, True, 'original-request')
        first = next(body)
        self.assertEqual(first, WIRE[:1])
        socket.sendall.assert_not_called()
        tail = next(body)
        self.assertEqual(first + tail, WIRE)
        body.close()  # No next() after the final offered chunk.
        self.assertEqual(events, ['message', 'waiting', 'released', 'tail'])
        socket.sendall.assert_called_once_with(b'm')
        socket.recv.assert_called_once_with(1)

    def test_sync_full_message_keeps_original_bytes_and_one_gate(self):
        events, socket = [], control()
        self.assertEqual(list(gated_sync(chunks, socket, events, False, 'original-request')), [WIRE])
        self.assertEqual(events, ['message', 'waiting', 'released'])
        socket.sendall.assert_called_once_with(b'm')
        socket.recv.assert_called_once_with(1)


class AsyncRejectionBodyTests(unittest.IsolatedAsyncioTestCase):
    async def test_async_consumer_can_close_after_taking_the_tail_without_another_poll(self):
        events, socket = [], control()
        body = gated_async(async_chunks, socket, events, True, 'original-request')
        first = await anext(body)
        self.assertEqual(first, WIRE[:1])
        socket.sendall.assert_not_called()
        tail = await anext(body)
        self.assertEqual(first + tail, WIRE)
        await body.aclose()  # No anext() after the final offered chunk.
        self.assertEqual(events, ['message', 'waiting', 'released', 'tail'])
        socket.sendall.assert_called_once_with(b'm')
        socket.recv.assert_called_once_with(1)

    async def test_async_full_message_keeps_original_bytes_and_one_gate(self):
        events, socket = [], control()
        result = [chunk async for chunk in gated_async(async_chunks, socket, events, False, 'original-request')]
        self.assertEqual(result, [WIRE])
        self.assertEqual(events, ['message', 'waiting', 'released'])
        socket.sendall.assert_called_once_with(b'm')
        socket.recv.assert_called_once_with(1)


if __name__ == '__main__':
    unittest.main()
