"""Controlled encoder scheduling; offered bytes do not prove socket delivery."""
import asyncio


def gated_sync(original_sync, control, events, partial, *args):
    for chunk in original_sync(*args):
        events.append('message')
        yield chunk[:1] if partial else chunk
        events.append('waiting')
        control.sendall(b'm')
        assert control.recv(1) == b'x', 'response completion gate missing'
        events.append('released')
        if partial:
            # Credit this offer before yielding: an early response can make
            # the transport close the iterator without polling it again.
            events.append('tail')
            yield chunk[1:]


async def gated_async(original_async, control, events, partial, *args, settled=None):
    control.setblocking(False)
    loop = asyncio.get_running_loop()
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
            # A decoded early rejection may cancel this pending upload before
            # the gate read resumes. That is not an offered or delivered tail.
            events.append('cancelled')
            raise
        finally:
            if settled is not None:
                settled.set()
        if partial:
            yield chunk[1:]
