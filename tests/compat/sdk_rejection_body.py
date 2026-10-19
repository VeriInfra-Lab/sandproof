"""Controlled encoder scheduling; offered bytes do not prove socket delivery."""
import asyncio


async def finish_gate(control, *, cancelled):
    # A cancelled sock_recv may already have consumed x without resuming its
    # generator. Keep the socket open through the sender's EOF in either case.
    control.setblocking(False)
    loop = asyncio.get_running_loop()
    remaining = b''
    while chunk := await loop.sock_recv(control, 2):
        remaining += chunk
        assert cancelled and remaining == b'x', 'unexpected response completion gate bytes'


def gated_sync(original_sync, control, events, partial, *args, settled=None):
    try:
        for chunk in original_sync(*args):
            events.append('message')
            yield chunk[:1] if partial else chunk
            events.append('waiting')
            control.sendall(b'm')
            assert control.recv(1) == b'x', 'response completion gate missing'
            events.append('released')
            if partial:
                # An offer does not imply that the consumer polls again.
                events.append('tail')
                yield chunk[1:]
    finally:
        if settled is not None:
            settled.set()


async def gated_async(original_async, control, events, partial, *args, settled=None):
    control.setblocking(False)
    loop = asyncio.get_running_loop()
    try:
        async for chunk in original_async(*args):
            events.append('message')
            yield chunk[:1] if partial else chunk
            events.append('waiting')
            try:
                await loop.sock_sendall(control, b'm')
                assert await loop.sock_recv(control, 1) == b'x', 'response completion gate missing'
                events.append('released')
                if partial:
                    events.append('tail')
            except asyncio.CancelledError:
                # Cancellation before the gate resumes offers no tail.
                events.append('cancelled')
                raise
            if partial:
                yield chunk[1:]
    finally:
        if settled is not None:
            settled.set()
