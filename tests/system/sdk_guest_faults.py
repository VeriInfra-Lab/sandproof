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
from sdk_guest_environment import mutate, pristine_control


def python(program):
    return '/usr/bin/python3 -I -c ' + shlex.quote(program)


def prepare_directory(vm, directory):
    # Minimal accepted images need not include /var/tmp. Only the parent is
    # idempotent: an existing per-probe leaf must fail, never reuse old effects.
    parent = str(Path(directory).parent)
    vm.commands.run('mkdir -p -- ' + shlex.quote(parent)
                    + ' && mkdir -m 0700 -- ' + shlex.quote(directory),
                    user='root', timeout=15)


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
    with raw_start(client, vm, 'printf once >> ' + shlex.quote(marker) + '; sleep 2') as response:
        assert response.status_code == 200
        # Keep the stream open until the real effect is observed independently,
        # then discard it without reading a PID. Closing immediately could
        # legitimately cancel preparation before execution and miss this fault.
        wait_for(lambda: vm.files.exists(marker), bool)
        assert vm.files.read(marker) == 'once'
    assert vm.files.read(marker) == 'once'
    assert vm.commands.run('printf healthy').stdout == 'healthy'
    assert vm.files.read(marker) == 'once'
    return dict(start_posts=1, start_pid_observed=False, effects=1)


RESTART = r'''import http.client,json,os,pathlib,select,signal,time
root=pathlib.Path(ROOT)
config=json.loads((root/'restart.json').read_text())
with open(config['init_path'],'rb') as source: init=source.read(65537)
assert 0<len(init)<=65536
assert set(json.loads(init)) <= {'accessToken','defaultUser','defaultWorkdir','envVars'}
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
            connection.request('POST','/init',init,
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


def restart_result(vm, path, diagnostics):
    def observed():
        try:
            return vm.files.read(path, user='root', request_timeout=2)
        except Exception as error:
            diagnostics['result_read_errors'] = diagnostics.get('result_read_errors', 0) + 1
            diagnostics['last_result_read_error'] = type(error).__name__[:128]
            return None
    return json.loads(wait_for(observed, lambda value: value is not None, seconds=30))


def restart_old_pid(vm, directory, candidate, init_path, diagnostics):
    before = asyncio.run(observe_candidate(vm, candidate, asynchronous=False))
    marker = directory + '/old-effects'
    old = vm.commands.run('printf once >> ' + shlex.quote(marker) + '; exec sleep 120',
                          user='root', background=True, timeout=120)
    old.disconnect()
    wait_for(lambda:vm.files.exists(marker),bool)
    assert vm.files.read(marker)=='once'
    # The protected harness staged this VM's original native init payload.
    # The SDK public gateway bearer is deliberately absent from this config.
    vm.files.write(directory + '/restart.json', json.dumps(dict(
        daemon=before['observation']['daemon'], command_pid=old.pid, init_path=init_path)), user='root')
    vm.commands.run(python(RESTART.replace('ROOT', repr(directory))), user='root', timeout=15)
    wait_for(lambda: vm.files.exists(directory + '/restart-ready'), bool)
    try:
        vm.commands.run('touch ' + shlex.quote(directory + '/restart-go'), timeout=15, request_timeout=15)
    except Exception:
        pass  # This one trigger may lose its reply; never send it a second time.
    result = restart_result(vm, directory + '/restart-result', diagnostics)
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
with open(p+'/ballast','wb',buffering=0) as f:
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
    command = 'printf once >> '+shlex.quote(marker)+'; while ! test -e '+shlex.quote(gate)+'; do sleep .05; done; '
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
    directory='/var/tmp/expiry-fault-'+uuid.uuid4().hex
    prepare_directory(vm, directory)
    ready,gate=directory+'/armed',directory+'/go'
    program = ('import os,signal,pathlib,time\npid='+str(daemon['pid'])+'\nfd=os.pidfd_open(pid)\n'
        +'assert int(pathlib.Path("/proc/"+str(pid)+"/stat").read_text().rsplit(")",1)[1].split()[19])=='
        +str(daemon['start_ticks'])+'\nos.rename("/opt/sandproof/state/pid-claims.bin",'
        +'"/opt/sandproof/state/original-pid-claims.test-backup")\n'
        +'assert not pathlib.Path("/opt/sandproof/state/pid-claims.bin").exists()\n'
        +'pathlib.Path('+repr(ready)+').write_text("armed")\ndeadline=time.monotonic()+30\n'
        +'while not pathlib.Path('+repr(gate)+').exists():\n'
        +'    assert time.monotonic()<deadline\n    time.sleep(.05)\n'
        +'signal.pidfd_send_signal(fd,signal.SIGKILL)\n')
    handle=vm.commands.run(python(program),user='root',background=True,timeout=30)
    handle.disconnect()
    wait_for(lambda:vm.files.exists(ready,user='root'),bool)
    assert vm.files.read(ready,user='root')=='armed'
    vm.set_timeout(8)
    try:
        vm.files.write(gate,'go',user='root',request_timeout=3)
    except Exception:
        pass  # The one release write can lose its reply; never replay it.
    assert vm.get_info().state=='running', 'expiry cannot stand in for a service fault'
    def unavailable():
        status=client.get('/health',headers={'X-Access-Token':vm._envd_access_token},timeout=1).status_code
        assert status in (200,502,503), 'expired/unauthorized/rate-limited is not the injected service fault'
        return status
    wait_for(unavailable,lambda status:status in (502,503),seconds=5)
    def expired():
        try:
            vm.get_info()
            return False
        except SandboxNotFoundException:
            return True
    wait_for(expired, bool, seconds=30)
    response = client.get('/health', headers={'X-Access-Token':vm._envd_access_token})
    assert response.status_code == 401
    return dict(fault_armed=True,unavailable_while_running=True,pid_file_missing=True,
                expiry_observed=True,old_token_rejected=True)


def exercise(args):
    report = dict(sdk_version='2.51.0', status='failed', faults=[],
        scope='selected guest faults; physical cleanup/host budgets must be confirmed by original harness')
    try:
        template, candidate = guest_candidate(args)
        assert candidate is not None
        from _guest_restart_fixture import load
        fixture = load(args.restart_fixture, template, candidate[0])
        owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
        settings=json.loads(Path(args.guest_configuration).read_text())
        for name in ('lost-start', 'restart-old-pid', 'full-filesystem', 'slow-output', 'registry-reclaim', 'lifecycle', 'unavailable-expiry'):
            vm = control = None
            row = dict(name=name, status='incomplete', diagnostics={}, cleanup={})
            report['faults'].append(row)
            primary = None
            cleanup_errors = []
            try:
                if name == 'restart-old-pid':
                    vm = Sandbox.connect(fixture['sandbox_id'], **owner)
                    assert vm.sandbox_id == fixture['sandbox_id']
                else:
                    vm = Sandbox.create(template=template, timeout=300, allow_internet_access=False, **owner)
                row['sandbox_id'] = vm.sandbox_id
                before = asyncio.run(observe_candidate(vm, candidate, asynchronous=False))
                if name in ('restart-old-pid','unavailable-expiry'):
                    control=Sandbox.create(template=template,timeout=300,allow_internet_access=False,**owner)
                    row['control_sandbox_id'] = control.sandbox_id
                directory = '/var/tmp/envd-fault-' + uuid.uuid4().hex
                prepare_directory(vm, directory)
                with httpx.Client(base_url=args.sandbox_url, trust_env=False, timeout=30) as client:
                    if name == 'lost-start': details = lost_start(vm, directory, client)
                    elif name == 'restart-old-pid':
                        details = restart_old_pid(vm, directory, candidate, fixture['init_path'], row['diagnostics'])
                    elif name == 'full-filesystem': details = full_filesystem(vm, directory)
                    elif name == 'slow-output': details = slow_output(vm, directory, client)
                    elif name == 'registry-reclaim': details = registry_reclaim(vm,directory,candidate,settings)
                    elif name == 'lifecycle':
                        details = lifecycle(vm, owner, client)
                        details['environment'] = mutate(vm, directory)
                        # A NEW VM created after mutation exercises the original
                        # immutable cache, not a second already-open filesystem.
                        control = Sandbox.create(template=template, timeout=300, allow_internet_access=False, **owner)
                        assert control.sandbox_id != vm.sandbox_id
                        row['control_sandbox_id'] = control.sandbox_id
                        asyncio.run(observe_candidate(control, candidate, asynchronous=False))
                        details['environment']['fresh_control_pristine'] = pristine_control(control, details['environment']['nonce'])
                        after = asyncio.run(observe_candidate(vm, candidate, asynchronous=False))
                        assert after == before, 'S03 changed the original daemon identity'
                        details['environment']['daemon_unchanged'] = True
                    else: details = unavailable_expiry(vm, owner, client, candidate)
                if control is not None:
                    assert control.commands.run('printf unaffected',timeout=15).stdout=='unaffected'
                row.update(status='pass', details=details)
            except BaseException as error:
                primary = error
                row.update(status='failed', failure_type=type(error).__name__[:128])
            finally:
                for role, instance in (('fault', vm), ('control', control)):
                    if instance is None:
                        continue
                    try:
                        acknowledged = instance.kill()
                        row['cleanup'][role] = dict(acknowledged=acknowledged)
                        assert acknowledged is True or (role == 'fault' and name == 'unavailable-expiry')
                    except BaseException as error:
                        row['cleanup'][role] = dict(error_type=type(error).__name__[:128])
                        cleanup_errors.append(error)
                if cleanup_errors:
                    row['status'] = 'failed'
                    raise BaseExceptionGroup('fault probe and original cleanup failed',
                        ([primary] if primary is not None else []) + cleanup_errors)
                if primary is not None:
                    raise primary
        report['status'] = 'pass'
    except BaseException as error:
        report['failure_type'] = type(error).__name__[:128]
        raise
    finally:
        # Bounded case records survive failed creation, observation and cleanup.
        # Exception messages, credentials and arbitrary guest output stay out.
        print(json.dumps(report, indent=2), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    guest_arguments(parser)
    parser.add_argument('--restart-fixture', required=True)
    exercise(parser.parse_args())
