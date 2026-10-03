import subprocess
import unittest
import urllib.error
from unittest.mock import MagicMock, patch
from tools import check_ci_isolation as probe


class IsolationProbeTests(unittest.TestCase):
    def invoke(self, *, uid=1000, sudo=1, groups=(), exists=False,
               host_connect=False, response=None):
        opener = MagicMock()
        opener.open.side_effect = (response if response is not None else
            urllib.error.HTTPError('http://test', 403, 'Forbidden', {}, None))
        socket_result = MagicMock() if host_connect else OSError('blocked')
        with patch.object(probe.os, 'geteuid', return_value=uid), \
             patch.object(probe.os, 'getgroups', return_value=groups), \
             patch.object(probe.grp, 'getgrgid', side_effect=lambda _: type('G', (), {'gr_name': 'sudo'})()), \
             patch.object(probe.subprocess, 'run', return_value=subprocess.CompletedProcess([], sudo)), \
             patch.object(probe.Path, 'exists', return_value=exists), \
             patch.object(probe.socket, 'create_connection', side_effect=socket_result), \
             patch.object(probe.urllib.request, 'build_opener', return_value=opener):
            probe.main()
        return opener

    def test_denials_are_established_not_inferred_from_timeout(self):
        self.assertEqual(self.invoke().open.call_count, 3)
        with self.assertRaisesRegex(RuntimeError, 'refusal was not established'):
            self.invoke(response=urllib.error.URLError('timeout'))

    def test_root_sudo_and_privileged_groups_refuse(self):
        for values in ({'uid': 0}, {'sudo': 0}, {'groups': [27]}):
            with self.subTest(values=values), self.assertRaises(RuntimeError):
                self.invoke(**values)

    def test_host_sharing_and_host_ssh_refuse(self):
        for values in ({'exists': True}, {'host_connect': True}):
            with self.subTest(values=values), self.assertRaises(RuntimeError):
                self.invoke(**values)

    def test_allowed_or_wrong_status_egress_refuses(self):
        for response in (lambda *_args, **_kwargs: MagicMock(),
                         urllib.error.HTTPError('http://test', 502, 'bad gateway', {}, None)):
            with self.subTest(response=response), self.assertRaises(RuntimeError):
                self.invoke(response=response)


if __name__ == '__main__':
    unittest.main()
