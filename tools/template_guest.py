"""Protected Rust payload for the original disposable-VM template worker.

Loading/staging only reads operator files. Installation runs the pinned check
command only through the worker's original deadline/log owner, inside its VM.
"""
import json
import os
from pathlib import Path
import stat

if __package__:
    from . import guest_artifacts as artifacts
    from .build_base_guest import verified_copy
    from .check_kvm_host import require_keys, unique_keys
    from .system_acceptance import private_path
else:
    import guest_artifacts as artifacts
    from build_base_guest import verified_copy
    from check_kvm_host import require_keys, unique_keys
    from system_acceptance import private_path

PLATFORM = '/opt/sandproof'
CONFIGURATION = 'guest-config.json'
PROFILE_KEYS = {'release', 'manifest_sha256', 'configuration', 'configuration_sha256'}

# A marker belongs to a new immutable image, never to a daemon restart. Consume
# the permission BEFORE initialize; an unknown result never permits re-create.
BOOTSTRAP = '''$B mkdir -m 700 /run/sandproof-guest
$B mkdir -m 700 /run/sandproof-guest/gates
if test -e /opt/sandproof/new-vm || test -L /opt/sandproof/new-vm; then
    test -f /opt/sandproof/new-vm
    test ! -L /opt/sandproof/new-vm
    test ! -e /opt/sandproof/initialization-attempted
    test ! -L /opt/sandproof/initialization-attempted
    $B mv /opt/sandproof/new-vm /opt/sandproof/initialization-attempted
    $B sync
    /opt/sandproof/sandproof-guest-service initialize --config /opt/sandproof/guest-config.json
fi
'''


def load_profile(path):
    """No execution, unpacking or image reads on the supervisor/staging host."""
    return _load_profile(path, artifacts.template_identity)


def load_runtime_profile(path):
    """The disposable worker receives runtime bytes, not the source archive."""
    return _load_profile(path, artifacts.runtime_template_identity)


def _load_profile(path, identify):
    path = private_path(path)
    _, raw = artifacts.fingerprint(path, artifacts.MANIFEST_LIMIT, content=True)
    value = json.loads(raw, object_pairs_hook=unique_keys)
    require_keys(value, PROFILE_KEYS, 'protected Rust guest profile')
    identity = identify(value['release'], value['manifest_sha256'],
                        value['configuration'], value['configuration_sha256'])
    # Read through the same bounded primitive, not an unbounded JSON reopen.
    observed, raw = artifacts.fingerprint(Path(value['configuration']), artifacts.CONFIGURATION_LIMIT, content=True)
    if observed != identity['configuration']:
        raise ValueError('guest configuration changed during profile load')
    config = json.loads(raw, object_pairs_hook=unique_keys)
    paths = config.get('paths')
    if (type(paths) is not dict or type(paths.get('owner_uid')) is not int
            or paths['owner_uid'] != 0 or config.get('listen') != '0.0.0.0:49983'
            or paths.get('worker') != PLATFORM + '/' + artifacts.BINARIES['worker']
            or paths.get('pid_directory') != PLATFORM + '/state'
            or paths.get('gate_directory') != '/run/sandproof-guest/gates'):
        raise ValueError('Rust guest configuration must use the fixed platform layout')
    return value, identity


def install(root, platform, profile, identity, commands, boot, agent):
    """Install into the new reserved platform directory, then check/re-read.

    This function does not mount images or initialize PID state. The original
    build entry has already enforced root in its assigned disposable Linux VM.
    """
    if commands is None:
        raise ValueError('original bounded worker command owner required')
    # The daemon's trusted path walk also checks these image ancestors. Refuse
    # a writable/non-owned parent instead of editing unrelated image ownership.
    for directory in (root, root / 'opt', platform):
        meta = directory.lstat()
        if (not stat.S_ISDIR(meta.st_mode) or meta.st_uid != os.geteuid()
                or meta.st_mode & 0o022):
            raise ValueError('untrusted Rust guest platform ancestor')
    release = Path(profile['release'])
    payloads = [(release / name, name, identity['artifacts'][role], 0o500)
                for role, name in artifacts.BINARIES.items()]
    manifest = release / 'manifest.json'
    observed = artifacts.fingerprint(manifest, artifacts.MANIFEST_LIMIT)
    if observed['sha256'] != identity['manifest_sha256']:
        raise ValueError('guest release manifest changed before install')
    payloads.extend([(Path(profile['configuration']), CONFIGURATION, identity['configuration'], 0o400),
                     (manifest, 'guest-manifest.json', observed, 0o400)])
    for source, name, expected, mode in payloads:
        verified_copy(source, platform / name, expected)
        (platform / name).chmod(mode)
    # Static schema/budget validation belongs to the actual pinned daemon. Its
    # check entry does not open worker/PID/MMDS paths, bind or start user work.
    commands.run([str(platform / artifacts.BINARIES['service']), 'check', '--config',
                  str(platform / CONFIGURATION)])
    for _, name, expected, mode in payloads:
        if artifacts.fingerprint(platform / name, expected['bytes'], executable=bool(mode & 0o100)) != expected:
            raise ValueError('installed guest payload changed')
    (platform / 'state').mkdir(mode=0o700)
    marker = platform / 'new-vm'
    with marker.open('xb') as output:
        os.fchmod(output.fileno(), 0o600)
        output.write(b'new immutable template VM\n')
    ready = '$B touch /run/sandproof-boot-ready\n'
    old_exec = 'exec /opt/sandproof/envd -verbose -no-cgroups\n'
    if boot.count(ready) != 1 or agent.count(old_exec) != 1:
        raise ValueError('platform boot contract changed')
    return (boot.replace(ready, BOOTSTRAP + ready),
            agent.replace(old_exec, 'exec /opt/sandproof/sandproof-guest-service serve --config /opt/sandproof/guest-config.json\n'))
