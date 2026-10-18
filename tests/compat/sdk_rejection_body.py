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


async def gated_async(original_async, control, events, partial, *args):
    async for chunk in original_async(*args):
        events.append('message')
        yield chunk[:1] if partial else chunk
        events.append('waiting')
        control.sendall(b'm')
        assert await asyncio.to_thread(control.recv, 1) == b'x', 'response completion gate missing'
        events.append('released')
        if partial:
            # Credit this offer before yielding: an early response can make
            # the transport close the iterator without polling it again.
            events.append('tail')
            yield chunk[1:]
