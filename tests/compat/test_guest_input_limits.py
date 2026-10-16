"""Pinned input-budget consumer checks; no product VM or KVM verdict."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.exceptions import RateLimitException, TimeoutException

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_guest_input_limits as probe


class InputLimitsTests(unittest.IsolatedAsyncioTestCase):
    def test_budget_is_read_from_the_pinned_actual_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'configuration.json'
            for value in (4096, 0, True, 65537):
                data = json.dumps(dict(commands=dict(owners=dict(stdin_bytes=value)))).encode()
                path.write_bytes(data)
                args = SimpleNamespace(guest_configuration=str(path),
                    guest_configuration_sha256=hashlib.sha256(data).hexdigest())
                if value == 4096:
                    self.assertEqual(probe.configured_limit(args), value)
                    args.guest_configuration_sha256 = '0' * 64
                with self.subTest(value=value), self.assertRaises(AssertionError):
                    probe.configured_limit(args)

    async def test_only_the_exact_observed_capacity_error_counts_as_rejection(self):
        for code, message in [(Code.RESOURCE_EXHAUSTED, probe.INPUT_LIMIT),
                              (Code.RESOURCE_EXHAUSTED, 'other policy'),
                              (Code.DEADLINE_EXCEEDED, probe.INPUT_LIMIT)]:
            error = RateLimitException(message + ': Rate limit exceeded, please try again later.')
            error.__context__ = ConnectError(code, message)
            call = AsyncMock(side_effect=error)
            if code is Code.RESOURCE_EXHAUSTED and message == probe.INPUT_LIMIT:
                self.assertEqual(await probe.rejected(call, 42, b'x', messages=(probe.INPUT_LIMIT,)), message)
            else:
                with self.assertRaises(AssertionError):
                    await probe.rejected(call, 42, b'x', messages=(probe.INPUT_LIMIT,))
            call.assert_awaited_once_with(42, b'x', request_timeout=15)
        call = AsyncMock(side_effect=TimeoutException('not quota evidence'))
        with self.assertRaises(TimeoutException):
            await probe.rejected(call, 42, b'x', messages=(probe.INPUT_LIMIT,))
        call.assert_awaited_once()


if __name__ == '__main__':
    unittest.main()
