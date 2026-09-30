"""Check driver-death cleanup with synthetic processes, no browser or task code."""
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import unittest

from benchmarks.guest_chromium import pidfd_open, pidfd_signal


@unittest.skipUnless(sys.platform == 'linux', 'Linux process lifecycle required')
class BrowserLifecycleTests(unittest.TestCase):
    def test_driver_death_reaps_browser_and_detached_descendant(self):
        with tempfile.TemporaryDirectory() as temp:
            log = Path(temp) / 'lifecycle.jsonl'
            browser = (
                'import json,os,subprocess,sys,time\n'
                'p=subprocess.Popen([sys.executable,"-c","import time; time.sleep(60)"],start_new_session=True)\n'
                'print(json.dumps({"browser":os.getpid(),"detached":p.pid,"wrapper":os.getppid()}),flush=True)\n'
                'time.sleep(60)\n')
            wrapper = ('import os\nfrom benchmarks.guest_chromium import supervise\n'
                       + f'supervise({[sys.executable, "-c", browser]!r},os.getppid(),{str(log)!r})\n')
            driver = ('import subprocess,sys\n'
                      + f'p=subprocess.Popen({[sys.executable, "-c", wrapper]!r})\n'
                      + 'sys.stdin.read()\n')
            process = subprocess.Popen([sys.executable, '-c', driver], stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            known = {}
            handles = {}
            try:
                if not select.select([process.stdout], [], [], 10)[0]:
                    diagnostic = (os.read(process.stderr.fileno(), 4096).decode()
                                  if select.select([process.stderr], [], [], 0)[0] else '')
                    self.fail('fixture startup timed out: ' + diagnostic)
                known = json.loads(process.stdout.readline())
                handles = {name: pidfd_open(pid) for name, pid in known.items()}
                process.stdin.close()  # driver exits while its browser is still running
                self.assertEqual(process.wait(timeout=5), 0)
                self.assertTrue(select.select([handles['wrapper']], [], [], 10)[0], 'launcher failed to finish cleanup')
                record = json.loads(log.read_text())
                self.assertEqual(record['reason'], 'driver_exit')
                self.assertEqual(record['remaining_owned_children'], [])
                self.assertFalse(Path('/proc', str(known['browser'])).exists())
                self.assertFalse(Path('/proc', str(known['detached'])).exists())
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
                for fd in handles.values():
                    try:
                        pidfd_signal(fd, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    finally:
                        os.close(fd)
                process.stdout.close()
                process.stderr.close()


if __name__ == '__main__':
    unittest.main()
