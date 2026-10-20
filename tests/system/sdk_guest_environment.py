"""S03 on a fresh Rust VM: offline package install and private system writes."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import hashlib
import json
from pathlib import Path
import shlex
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.guest_artifacts import fingerprint
from tools.system_acceptance import json_file

PACKAGE = 'sandproof-e06-dependency'
MODULE = '_sandproof_e06_dependency'
MARKER = '/etc/sandproof-e06-'

PRISTINE = r'''import importlib.util,json,os,pathlib,subprocess,sys
marker,module,package=sys.argv[1:]
assert not os.path.lexists(marker)
assert importlib.util.find_spec(module) is None
query=subprocess.run(['dpkg-query','-W','-f=${Status}',package],capture_output=True)
assert query.returncode==1
print(json.dumps({'pristine':True}))
'''

PREPARE = r'''import importlib.util,json,os,pathlib,subprocess,sys
root,nonce,module,package=pathlib.Path(sys.argv[1]),*sys.argv[2:]
assert os.geteuid()==0
assert json.loads(pathlib.Path('/opt/sandproof/guest-manifest.json').read_text())['kind']=='rust-guest-v1'
assert importlib.util.find_spec(module) is None
source=root/'package'
(source/'DEBIAN').mkdir(parents=True,mode=0o755)
(source/'DEBIAN/control').write_text('Package: '+package+'\nVersion: 1.0\nArchitecture: all\nMaintainer: Sandproof acceptance\nDescription: isolated acceptance dependency fixture\n')
library=source/'usr/lib/python3/dist-packages'
library.mkdir(parents=True,mode=0o755)
(library/(module+'.py')).write_text('VALUE = '+repr(nonce)+'\n')
for path in source.rglob('*'):
    path.chmod(0o755 if path.is_dir() else 0o644)
result=subprocess.run(['dpkg-deb','--build','--root-owner-group',str(source),str(root/'dependency.deb')],capture_output=True,timeout=10)
assert result.returncode==0
assert len(result.stdout)+len(result.stderr)<=4096
print(json.dumps({'prepared':True}))
'''

IMPORTED = r'''import importlib,json,pathlib,sys
module,nonce=sys.argv[1:]
loaded=importlib.import_module(module)
assert loaded.VALUE==nonce
assert pathlib.Path(loaded.__file__)==pathlib.Path('/usr/lib/python3/dist-packages')/(module+'.py')
print(json.dumps({'imported':True}))
'''


def _command(vm, program, *arguments, user='root'):
    command = '/usr/bin/python3 -I -c ' + shlex.quote(program)
    command += ''.join(' ' + shlex.quote(str(arg)) for arg in arguments)
    result = vm.commands.run(command, user=user, timeout=15, request_timeout=15)
    assert result.exit_code == 0 and not result.stderr and len(result.stdout.encode()) <= 4096
    return json.loads(result.stdout)


def mutate(vm, directory):
    nonce = directory.rsplit('-', 1)[-1]
    assert len(nonce) == 32 and all(c in '0123456789abcdef' for c in nonce)
    marker = MARKER + nonce
    assert _command(vm, PRISTINE, marker, MODULE, PACKAGE) == {'pristine':True}
    assert _command(vm, PREPARE, directory, nonce, MODULE, PACKAGE) == {'prepared':True}
    result = vm.commands.run('dpkg --install ' + shlex.quote(directory + '/dependency.deb'),
                             user='root', timeout=15, request_timeout=15)
    assert result.exit_code == 0 and len(result.stdout.encode()) + len(result.stderr.encode()) <= 4096
    for user in ('root', 'ubuntu'):
        assert _command(vm, IMPORTED, MODULE, nonce, user=user) == {'imported':True}
    payload = b'private-system\x00\x80' + nonce.encode()
    vm.files.write(marker, payload, user='root', request_timeout=15)
    assert vm.files.read(marker, format='bytes', user='root', request_timeout=15) == payload
    return dict(installed=True, system_file=True, nonce=nonce,
                system_file_bytes=len(payload), system_file_sha256=hashlib.sha256(payload).hexdigest())


def pristine_control(control, nonce):
    assert _command(control, PRISTINE, MARKER + nonce, MODULE, PACKAGE) == {'pristine':True}
    return True


def cached_template(cache, template, expected):
    """Hash the actual shared runtime cache, never a guest-reported disk hash."""
    root = Path(cache['root']) / 'artifacts'
    matches = []
    for index, directory in enumerate(root.iterdir()):
        assert index < cache['max_entries'] and directory.is_dir() and not directory.is_symlink()
        manifest = json_file(directory / 'manifest.json', limit=1024**2)
        local = manifest['local']
        if local['template_id'] != template:
            continue
        disk = directory / 'rootfs.ext4'
        assert Path(local['rootfs']['path']) == disk
        pin = dict(bytes=local['rootfs']['size_bytes'], sha256=local['rootfs']['sha256'])
        assert pin == expected and pin['bytes'] <= cache['max_entry_bytes']
        assert fingerprint(disk, pin['bytes']) == pin
        meta = disk.lstat()
        matches.append(dict(artifact=pin, identity=[meta.st_dev,meta.st_ino,meta.st_size,
            meta.st_mtime_ns,meta.st_ctime_ns,meta.st_uid,meta.st_gid,meta.st_mode,meta.st_nlink]))
    assert len(matches) == 1, 'one actual cached original Rust template required'
    return matches[0]
