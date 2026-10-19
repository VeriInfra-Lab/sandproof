"""A request-body consumer may stop immediately after accepting the last chunk."""
import asyncio
import socket
import threading
import unittest
from unittest.mock import Mock
from sdk_rejection_body import finish_gate, gated_async, gated_sync


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
    def test_sync_tail_offer_does_not_settle_until_iterator_closes(self):
        events, settled = [], threading.Event()
        body = gated_sync(chunks, control(), events, True, 'original-request', settled=settled)
        self.assertEqual(next(body), WIRE[:1])
        self.assertFalse(settled.is_set())
        self.assertEqual(next(body), WIRE[1:])
        self.assertFalse(settled.is_set())
        body.close()
        self.assertTrue(settled.is_set())
        self.assertEqual(events, ['message', 'waiting', 'released', 'tail'])

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
    async def asyncSetUp(self):
        self.control, self.peer = socket.socketpair()
        self.control.setblocking(False)
        self.peer.setblocking(False)
        self.addCleanup(self.control.close)
        self.addCleanup(self.peer.close)

    async def test_cancelled_reader_keeps_gate_open_until_the_sender_finishes(self):
        # Upload cancellation leaves the release unread; the sender can still
        # complete its original write before the fixture closes the socket.
        waiter = asyncio.create_task(finish_gate(self.control, cancelled=True))
        self.peer.sendall(b'x')
        self.peer.shutdown(socket.SHUT_WR)
        await asyncio.wait_for(waiter, 5)

    async def test_gate_eof_after_release_was_consumed(self):
        self.peer.shutdown(socket.SHUT_WR)
        for cancelled in (False, True):
            await asyncio.wait_for(finish_gate(self.control, cancelled=cancelled), 5)

    async def test_gate_drain_rejects_wrong_or_duplicate_release(self):
        for data in (b'!', b'xx'):
            with self.subTest(data=data):
                self.peer.sendall(data)
                with self.assertRaisesRegex(AssertionError, 'unexpected response completion gate bytes'):
                    await asyncio.wait_for(finish_gate(self.control, cancelled=True), 5)

    async def test_completed_generator_cannot_leave_an_unread_release(self):
        self.peer.sendall(b'x')
        with self.assertRaisesRegex(AssertionError, 'unexpected response completion gate bytes'):
            await asyncio.wait_for(finish_gate(self.control, cancelled=False), 5)

    async def test_async_consumer_can_close_after_taking_the_tail_without_another_poll(self):
        events = []
        body = gated_async(async_chunks, self.control, events, True, 'original-request')
        first = await anext(body)
        self.assertEqual(first, WIRE[:1])
        with self.assertRaises(BlockingIOError):
            self.peer.recv(1)
        self.peer.sendall(b'x')
        tail = await anext(body)
        self.assertEqual(first + tail, WIRE)
        await body.aclose()  # No anext() after the final offered chunk.
        self.assertEqual(events, ['message', 'waiting', 'released', 'tail'])
        self.assertEqual(await asyncio.wait_for(asyncio.get_running_loop().sock_recv(self.peer, 1), 5), b'm')

    async def test_async_full_message_keeps_original_bytes_and_one_gate(self):
        events = []
        self.peer.sendall(b'x')
        result = [chunk async for chunk in gated_async(async_chunks, self.control, events, False, 'original-request')]
        self.assertEqual(result, [WIRE])
        self.assertEqual(events, ['message', 'waiting', 'released'])
        self.assertEqual(await asyncio.wait_for(asyncio.get_running_loop().sock_recv(self.peer, 1), 5), b'm')

    async def test_async_tail_offer_does_not_settle_until_iterator_closes(self):
        events, settled = [], asyncio.Event()
        body = gated_async(async_chunks, self.control, events, True,
                           'original-request', settled=settled)
        self.assertEqual(await anext(body), WIRE[:1])
        self.peer.sendall(b'x')
        self.assertEqual(await anext(body), WIRE[1:])
        self.assertFalse(settled.is_set())
        await body.aclose()
        self.assertTrue(settled.is_set())
        self.assertEqual(events, ['message', 'waiting', 'released', 'tail'])

    async def test_cancelled_pending_gate_is_observed_without_offering_a_tail(self):
        for released in (False, True):
            with self.subTest(released=released):
                events, settled = [], asyncio.Event()
                body = gated_async(async_chunks, self.control, events, True,
                                   'original-request', settled=settled)
                self.assertEqual(await anext(body), WIRE[:1])
                pending = asyncio.create_task(anext(body))
                self.assertEqual(await asyncio.wait_for(asyncio.get_running_loop().sock_recv(self.peer, 1), 5), b'm')
                self.assertFalse(settled.is_set())
                if released:
                    self.peer.sendall(b'x')
                pending.cancel()  # Before the original reader is scheduled again.
                with self.assertRaises(asyncio.CancelledError):
                    await pending
                await asyncio.wait_for(settled.wait(), 1)
                await body.aclose()
                self.assertEqual(events, ['message', 'waiting', 'cancelled'])

    async def test_wrong_gate_byte_is_a_failure_not_cancellation_or_a_tail(self):
        events, settled = [], asyncio.Event()
        body = gated_async(async_chunks, self.control, events, True,
                           'original-request', settled=settled)
        await anext(body)
        self.peer.sendall(b'!')
        with self.assertRaisesRegex(AssertionError, 'response completion gate missing'):
            await anext(body)
        self.assertTrue(settled.is_set())
        self.assertEqual(events, ['message', 'waiting'])


if __name__ == '__main__':
    unittest.main()
