"""Selected #311 faults on fresh private Rust VMs, through the original SDK.

Native fault injection is confined to that VM. It is not a new public service
mode or protection against guest root. Unknown Start is never retried.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import asyncio
import base64
import json
import os
import shlex
import time
import uuid
from pathlib import Path

import httpx
from e2b import Sandbox
from e2b.exceptions import SandboxNotFoundException, RateLimitException, SandboxException
from sdk_command_input import call
from sdk_data_contract import guest_arguments, guest_candidate, observe_candidate
from sdk_server_smoke import options


def python(program):
    return '/usr/bin/python3 -I -c ' + shlex.quote(program)


def wait_for(observe, accept, *, seconds=20):
    deadline = time.monotonic() + seconds
    while True:
        result = observe()
        if accept(result):
            return result
        assert time.monotonic() < deadline, 'original condition remained unobserved'
        time.sleep(.05)  # Poll a specific real barrier, not a probabilistic race.


def raw_start(client, sandbox, command):
    request = json.dumps(dict(process=dict(cmd='/bin/sh', args=['-c', command]), stdin=False)).encode()
    return client.stream('POST', '/process.Process/Start',
        content=b'\x00' + len(request).to_bytes(4, 'big') + request,
        headers={'X-Access-Token':sandbox._envd_access_token,
                 'Authorization':'Basic ' + base64.b64encode(b'root:').decode(),
                 'Content-Type':'application/connect+json', 'Connect-Protocol-Version':'1',
                 'Connect-Timeout-Ms':'60000'})


def lost_start(vm, directory, client):
    marker = directory + '/unknown-start'
    # Read headers only: do NOT consume/obtain a Start PID. The effect, rather
    # than a guessed PID or a second Start, proves that execution happened.
    with raw_start(client, vm, 'printf once > ' + shlex.quote(marker) + '; sleep 2') as response:
        assert response.status_code == 200
    wait_for(lambda: vm.files.exists(marker), bool)
    assert vm.files.read(marker) == 'once'
    assert vm.commands.run('printf healthy').stdout == 'healthy'
    assert vm.files.read(marker) == 'once'
    return dict(start_posts=1, start_pid_observed=False, effects=1)


RESTART = r'''import http.client,json,os,pathlib,select,signal,time
root=pathlib.Path(ROOT)
config=json.loads((root/'restart.json').read_text())
daemon=config['daemon']; old_pid=config['command_pid']
pid=os.fork()
if pid: raise SystemExit(0)
os.setsid()
for fd in (0,1,2):
    try: os.close(fd)
    except OSError: pass
fd=os.open('/dev/null',os.O_RDWR)
for n in (0,1,2): os.dup2(fd,n)
try:
    def ticks(pid): return int(pathlib.Path('/proc/'+str(pid)+'/stat').read_text().rsplit(')',1)[1].split()[19])
    target=os.pidfd_open(daemon['pid'])
    old=os.pidfd_open(old_pid)
    assert ticks(daemon['pid'])==daemon['start_ticks']
    # Observe the actual fixed private file below instead of creating state.
    files=list(pathlib.Path('/opt/sandproof/state').iterdir())
    assert len(files)==1
    original=files[0].stat()
    saved=files[0].read_bytes()
    assert saved[52+old_pid//8] & (1<<(old_pid%8)), 'old PID was not durably claimed'
    session=int.from_bytes(saved[44:52],'little')
    (root/'restart-ready').write_text('ready')
    deadline=time.monotonic()+30
    while not (root/'restart-go').exists():
        assert time.monotonic()<deadline
        time.sleep(.05)
    signal.pidfd_send_signal(target,signal.SIGKILL)
    assert select.select([target],[],[],10)[0]
    assert not select.select([old],[],[],0)[0], 'old user process unexpectedly disappeared'
    initialized=False
    while time.monotonic()<deadline:
        connection=http.client.HTTPConnection('127.0.0.1',49983,timeout=1)
        try:
            connection.request('POST','/init',json.dumps({'accessToken':config['token'],'defaultUser':'root'}),
                               {'Content-Type':'application/json'})
            response=connection.getresponse(); response.read(1024)
            if response.status==204:
                initialized=True; break
        except OSError: pass
        finally: connection.close()
        time.sleep(.05)
    assert initialized, 'original daemon respawn/init remained unavailable'
    assert (files[0].stat().st_dev,files[0].stat().st_ino)==(original.st_dev,original.st_ino)
    current=files[0].read_bytes()
    assert current[52+old_pid//8] & (1<<(old_pid%8))
    assert int.from_bytes(current[44:52],'little')>session
    assert not select.select([old],[],[],0)[0]
    result={'status':'pass','old_process_alive':True,'same_pid_file':True}
except BaseException as error:
    result={'status':'failed','error_type':type(error).__name__}
(root/'restart-result').write_text(json.dumps(result))
'''


def restart_old_pid(vm, directory, candidate):
    before = asyncio.run(observe_candidate(vm, candidate, asynchronous=False))
    marker = directory + '/old-effects'
    old = vm.commands.run('printf once > ' + shlex.quote(marker) + '; exec sleep 120',
                          user='root', background=True, timeout=120)
    old.disconnect()
    wait_for(lambda:vm.files.exists(marker),bool)
    assert vm.files.read(marker)=='once'
    # Only this sandbox's existing SDK token enters its own private test file;
    # no center, host, App or CI credential is sent into the guest.
    vm.files.write(directory + '/restart.json', json.dumps(dict(
        daemon=before['observation']['daemon'], command_pid=old.pid, token=vm._envd_access_token)), user='root')
    vm.commands.run(python(RESTART.replace('ROOT', repr(directory))), user='root', timeout=15)
    wait_for(lambda: vm.files.exists(directory + '/restart-ready'), bool)
    try:
        vm.commands.run('touch ' + shlex.quote(directory + '/restart-go'), timeout=15, request_timeout=15)
    except Exception:
        pass  # This one trigger may lose its reply; never send it a second time.
    def observed():
        try:
            return vm.files.read(directory + '/restart-result', user='root', request_timeout=2)
        except Exception:
            return None
    result = json.loads(wait_for(observed, lambda value: value is not None, seconds=30))
    assert result['status'] == 'pass', result
    after = asyncio.run(observe_candidate(vm, candidate, asynchronous=False))
    assert before['identity'] == after['identity']
    assert before['observation']['daemon'] != after['observation']['daemon']
    assert vm.files.read(marker) == 'once'
    assert all(item.pid != old.pid for item in vm.commands.list())
    # The old user process is really alive, but new-session control must refuse
    # it. No connect, input or signal may adopt it merely by its numerical PID.
    refused = vm.commands.kill(old.pid)
    assert refused is False
    try:
        vm.commands.connect(old.pid, timeout=5)
    except Exception as error:
        assert type(error).__name__ in ('NotFoundException', 'ProcessNotFoundException')
    else:
        raise AssertionError('new daemon adopted old PID')
    assert vm.commands.run('kill -0 ' + str(old.pid) + '; printf healthy', user='root').stdout == 'healthy'
    return result


def full_filesystem(vm, directory):
    target = directory + '/full'
    vm.commands.run('mkdir ' + shlex.quote(target) + ' && mount -t tmpfs -o size=16m,nodev,nosuid,noexec tmpfs '
                    + shlex.quote(target), user='root', timeout=15)
    program = '''import errno,os
p=PATH
with open(p+'/ballast','wb') as f:
    try:
        while True: f.write(b'B'*65536); f.flush()
    except OSError as e:
        assert e.errno==errno.ENOSPC
    f.truncate(max(0,os.fstat(f.fileno()).st_size-65536))
'''.replace('PATH', repr(target))
    vm.commands.run(python(program), user='root', timeout=15)
    try:
        vm.files.write(target + '/partial', b'P' * (2*1024**2), user='root', request_timeout=15)
    except SandboxException as error:
        failure = type(error).__name__
    else:
        raise AssertionError('full filesystem upload falsely succeeded')
    actual = vm.files.read(target + '/partial', format='bytes', user='root')
    assert 0 < len(actual) < 2*1024**2 and actual == b'P' * len(actual)
    assert vm.commands.run('printf healthy', timeout=15).stdout == 'healthy'
    vm.commands.run('umount ' + shlex.quote(target), user='root', timeout=15)
    return dict(partial_bytes=len(actual), successful_upload=False, failure=failure, healthy=True)


def slow_output(vm, directory, client):
    marker = directory + '/output-start'
    gate = directory + '/output-go'
    command = 'printf once > '+shlex.quote(marker)+'; while ! test -e '+shlex.quote(gate)+'; do sleep .05; done; '
    command += 'head -c 16777216 /dev/zero; printf done > '+shlex.quote(directory+'/output-done')
    with raw_start(client, vm, command) as response:
        assert response.status_code == 200
        wait_for(lambda: vm.files.exists(marker), bool)
        vm.commands.run('touch '+shlex.quote(gate), timeout=15)
        assert vm.commands.run('printf independent', timeout=15).stdout == 'independent'
        # The first subscriber remains unread; a separate metadata/file request
        # must observe actual producer completion, not unblock it by draining.
        wait_for(lambda: vm.files.exists(directory+'/output-done'), bool, seconds=30)
    assert vm.files.read(marker) == 'once'
    assert vm.commands.run('printf after', timeout=15).stdout == 'after'
    return dict(output_bytes=16*1024**2, starts=1, other_request_healthy=True)


def lifecycle(vm, owner, client):
    assert vm.get_info().state == 'running'
    assert Sandbox.connect(vm.sandbox_id, timeout=180, **owner).sandbox_id == vm.sandbox_id
    vm.set_timeout(180)
    assert vm.commands.run('printf renewed', timeout=15).stdout == 'renewed'
    return dict(connected=True, renewed=True, running=True)


def registry_reclaim(vm, directory, candidate, settings):
    """Cross the real retained-record capacity twice; never retry unknown work."""
    capacity=min(settings['commands']['owners']['records'],settings['commands']['registry']['records'])
    assert type(capacity) is int and 0<capacity<=256, 'reviewed finite churn fixture required'
    retention=settings['commands']['registry']['retention']
    assert retention == {'secs':30,'nanos':0}
    observation=asyncio.run(observe_candidate(vm,candidate,asynchronous=False))
    daemon=observation['observation']['daemon']['pid']
    marker=directory+'/churn'
    vm.files.write(marker,'',user='root')
    delivered=refused=0
    samples=[]
    def execute(command):
        nonlocal refused
        try:
            return vm.commands.run(command,user='root',timeout=15,request_timeout=15)
        except RateLimitException:
            # This is an explicit admission refusal, not an uncertain Start.
            # Confirm no extra effect, wait the original 30-second retention,
            # then make a NEW attempt. No unknown request is replayed.
            refused+=1
            assert vm.files.read(marker,user='root')=='x'*delivered
            due=time.monotonic()+30
            while time.monotonic()<due:
                time.sleep(max(0,min(.2,due-time.monotonic())))
            return vm.commands.run(command,user='root',timeout=15,request_timeout=15)
    claims_before=vm.files.get_info('/opt/sandproof/state/pid-claims.bin',user='root').size
    for _ in range(2*capacity+2):
        result=execute('printf x >> '+shlex.quote(marker))
        assert result.exit_code==0 and result.stdout=='' and result.stderr==''
        delivered+=1
        if delivered in (1,capacity,2*capacity+2):
            # procfs pseudo-files have st_size==0. Observe them through the
            # native command, not a length-based HTTP file download.
            status=execute('cat /proc/'+str(daemon)+'/status').stdout
            assert len(status.encode())<=8192
            fields={line.split(':',1)[0]:line.split(':',1)[1].strip() for line in status.splitlines() if ':' in line}
            samples.append({key:int(fields[key].split()[0]) for key in ('VmRSS','VmHWM','Threads')})
            assert samples[-1]['VmHWM']*1024<=512*1024**2, 'daemon exceeds declared whole-guest memory envelope'
    assert delivered>2*capacity and vm.files.read(marker,user='root')=='x'*delivered
    assert vm.commands.list()==[]
    assert vm.files.get_info('/opt/sandproof/state/pid-claims.bin',user='root').size==claims_before
    return dict(record_capacity=capacity,completed=delivered,explicit_refusals=refused,
                original_retention_seconds=30,samples=samples,scope='finite churn and observed guest RSS, not an entire-process memory proof')


def unavailable_expiry(vm, owner, client, candidate):
    observation = asyncio.run(observe_candidate(vm, candidate, asynchronous=False))
    daemon = observation['observation']['daemon']
    vm.set_timeout(8)
    program = ('import os,signal,pathlib\npid='+str(daemon['pid'])+'\nfd=os.pidfd_open(pid)\n'
        +'assert int(pathlib.Path("/proc/"+str(pid)+"/stat").read_text().rsplit(")",1)[1].split()[19])=='
        +str(daemon['start_ticks'])+'\nos.rename("/opt/sandproof/state/pid-claims.bin",'
        +'"/opt/sandproof/state/original-pid-claims.test-backup")\nsignal.pidfd_send_signal(fd,signal.SIGKILL)\n')
    try:
        vm.commands.run(python(program), user='root', timeout=3, request_timeout=3)
    except Exception:
        pass
    else:
        raise AssertionError('killed daemon falsely completed its command')
    def expired():
        try:
            vm.get_info()
            return False
        except SandboxNotFoundException:
            return True
    wait_for(expired, bool, seconds=30)
    response = client.get('/health', headers={'X-Access-Token':vm._envd_access_token})
    assert response.status_code == 401
    return dict(daemon_killed=True,pid_file_missing=True,expiry_observed=True,old_token_rejected=True)


def exercise(args):
    template, candidate = guest_candidate(args)
    assert candidate is not None
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    settings=json.loads(Path(args.guest_configuration).read_text())
    rows = []
    for name in ('lost-start', 'restart-old-pid', 'full-filesystem', 'slow-output', 'registry-reclaim', 'lifecycle', 'unavailable-expiry'):
        vm = Sandbox.create(template=template, timeout=300, allow_internet_access=False, **owner)
        control=None
        row = dict(name=name, status='incomplete', sandbox_id=vm.sandbox_id)
        rows.append(row)
        try:
            asyncio.run(observe_candidate(vm, candidate, asynchronous=False))
            if name in ('restart-old-pid','unavailable-expiry'):
                control=Sandbox.create(template=template,timeout=300,allow_internet_access=False,**owner)
            directory = '/var/tmp/envd-fault-' + uuid.uuid4().hex
            vm.commands.run('mkdir -m 0700 '+shlex.quote(directory), user='root', timeout=15)
            with httpx.Client(base_url=args.sandbox_url, trust_env=False, timeout=30) as client:
                if name == 'lost-start': details = lost_start(vm, directory, client)
                elif name == 'restart-old-pid': details = restart_old_pid(vm, directory, candidate)
                elif name == 'full-filesystem': details = full_filesystem(vm, directory)
                elif name == 'slow-output': details = slow_output(vm, directory, client)
                elif name == 'registry-reclaim': details = registry_reclaim(vm,directory,candidate,settings)
                elif name == 'lifecycle': details = lifecycle(vm, owner, client)
                else: details = unavailable_expiry(vm, owner, client, candidate)
            if control is not None:
                assert control.commands.run('printf unaffected',timeout=15).stdout=='unaffected'
            row.update(status='pass', details=details)
        finally:
            try:
                acknowledged = vm.kill()
                assert acknowledged is True or name == 'unavailable-expiry'
            finally:
                if control is not None:
                    control_ack=control.kill()
                    assert control_ack is True
    print(json.dumps(dict(sdk_version='2.51.0', faults=rows,
        scope='selected guest faults; physical cleanup/host budgets must be confirmed by original harness'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    guest_arguments(parser)
    exercise(parser.parse_args())
