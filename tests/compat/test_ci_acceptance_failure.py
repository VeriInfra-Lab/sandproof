"""Intentional CI acceptance fault; this diagnostic branch must never merge."""
import unittest


class ExpectedCiFailure(unittest.TestCase):
    def test_intentional_sdk_gate_failure(self):
        self.fail('CI_NEGATIVE_sdk_contract_expected')
