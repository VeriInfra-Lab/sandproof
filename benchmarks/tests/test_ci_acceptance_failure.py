"""Intentional CI acceptance fault; this diagnostic branch must never merge."""
import unittest


class ExpectedCiFailure(unittest.TestCase):
    def test_intentional_harbor_gate_failure(self):
        self.fail('CI_NEGATIVE_harbor_inputs_expected')
