"""Pinned no-create DK recovery constructor; no VM or network request is sent."""
import importlib.metadata
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from e2b import Sandbox
import httpx
from packaging.version import Version

from sdk_support import SANDBOX, isolated_sdk_environment

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT))
sys.path.insert(0,str(ROOT/'tests/system'))
from _center_fault_cases import sdk_from_detail


class RecoveryConstructorContract(unittest.TestCase):
    def test_detail_recovers_actual_sdk_client_without_create_connect_renew_or_network(self):
        self.assertEqual(importlib.metadata.version('e2b'),'2.51.0')
        options=dict(api_url='http://127.0.0.1:12345',sandbox_url='http://127.0.0.1:12345',
            api_key='synthetic-owner-key',domain='fallback.invalid',retries=0,request_timeout=30)
        probe=SimpleNamespace(fetch=Mock(return_value=(200,json.dumps(SANDBOX).encode())),
                              sdk_options=Mock(return_value=options))
        with isolated_sdk_environment(), \
                patch.object(Sandbox,'create',side_effect=AssertionError('unexpected create')) as create, \
                patch.object(Sandbox,'connect',side_effect=AssertionError('unexpected renew')) as connect, \
                patch.object(Sandbox,'set_timeout',side_effect=AssertionError('unexpected renew')) as renew, \
                patch.object(httpx.Client,'send',side_effect=AssertionError('unexpected network')) as send:
            sandbox=sdk_from_detail(probe,SANDBOX['sandboxID'])
            self.addCleanup(sandbox._envd_api.close)
            self.assertEqual(sandbox.sandbox_id,SANDBOX['sandboxID'])
            self.assertEqual(sandbox.sandbox_domain,SANDBOX['domain'])
            self.assertEqual(sandbox._envd_version,Version(SANDBOX['envdVersion']))
            self.assertEqual(sandbox._envd_access_token,SANDBOX['envdAccessToken'])
            self.assertEqual(sandbox.envd_api_url,options['sandbox_url'])
            self.assertEqual(sandbox.connection_config.api_url,options['api_url'])
            self.assertEqual(sandbox.connection_config.api_key,options['api_key'])
            self.assertEqual(sandbox.connection_config.retries,0)
            self.assertEqual(sandbox.connection_config.request_timeout,30)
            self.assertEqual(sandbox._envd_api.headers['X-Access-Token'],SANDBOX['envdAccessToken'])
            for method in (create,connect,renew,send):
                method.assert_not_called()
        probe.fetch.assert_called_once_with('GET','/sandboxes/'+SANDBOX['sandboxID'])


if __name__=='__main__':
    unittest.main()
