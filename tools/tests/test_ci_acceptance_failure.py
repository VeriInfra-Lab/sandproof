"""Intentional CI acceptance fault; this diagnostic branch must never merge."""
import unittest


class ExpectedCiFailure(unittest.TestCase):
    def test_intentional_rust_gate_failure(self):
        self.fail('CI_NEGATIVE_rust_expected')
