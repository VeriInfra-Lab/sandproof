"""Build/publish one Rust template using the original private builder and gate."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import gzip
import json
import os
from pathlib import Path
import secrets
import time
import urllib.request

from _complete_registry_runtime import _save
from server_process_crash import fetch
from template_build import checked, upload
from tools.template_guest import load_profile
from tools.template_stage import file_hash, stage, write_sparse

HELPERS = ('template_worker.py', 'template_inputs.py', 'template_guest.py', 'guest_artifacts.py',
           'build_base_guest.py', 'build_ci_guest.py', 'check_kvm_host.py', 'system_acceptance.py')


def build(deployment, *, checkout, source, profile, kernel):
    """No alternate builder, local extraction or publication success shortcut."""
    pins, identity = load_profile(profile)
    config = json.loads(deployment.build_config.read_text())
    builder, builder_id = deployment.open_builder(timeout=900, internet=False)
    # Its original durable intent survives every exception. The caller fences
    # and settles this exact builder even when open/build/upload is unknown.
    checked(builder, 'mkdir -m 700 -p /input/tools /input/release /output')
    tools = {}
    for name, item in config['tools'].items():
        if name == 'regctl':
            continue  # This approved OCI archive has no registry acquisition.
        upload(builder, Path(item['path']), '/input/tools/' + name)
        tools[name] = dict(path='/input/tools/' + name, sha256=item['sha256'])
    checked(builder, 'chmod 755 /input/tools/*')
    for name in HELPERS:
        upload(builder, checkout / 'tools' / name, '/input/' + name)
    for name in ('manifest.json', 'sandproof-guest-service', 'sandproof-guest-worker'):
        upload(builder, Path(pins['release']) / name, '/input/release/' + name)
    upload(builder, Path(pins['configuration']), '/input/configuration.json')
    remote = dict(pins, release='/input/release', configuration='/input/configuration.json')
    builder.files.write('/input/rust-profile.json', json.dumps(remote), user='root')
    builder.files.write('/input/tools.json', json.dumps(tools), user='root')
    checked(builder, 'chmod 700 /input /input/release; chmod 500 /input/release/sandproof-*; '
                    'chmod 400 /input/release/manifest.json /input/configuration.json /input/rust-profile.json')
    upload(builder, source, '/input/source.tar.gz')
    limits = config['limits']
    request = dict(kind='oci', disk_mib=2048, min_free_mib=256,
        **{name: limits[name] for name in ('max_input_bytes', 'max_expanded_bytes',
                                        'max_entries', 'timeout_seconds', 'max_log_bytes')})
    builder.files.write('/input/request.json', json.dumps(request), user='root')
    boot = checked(builder, 'cat /proc/sys/kernel/random/boot_id').strip()
    # The original worker owns the build deadline; SDK waiting has the same
    # pre-existing bounded observation allowance, not a longer worker budget.
    checked(builder, 'python3 /input/template_worker.py --work /output/job --source /input/source.tar.gz '
        '--request /input/request.json --profile /input/tools.json --guest-boot-id ' + boot
        + ' --rust-guest-profile /input/rust-profile.json', timeout=650)
    status = json.loads(builder.files.read('/output/job/status.json', user='root'))
    _save(deployment.root / 'inputs' / 'rust-build-status.json', status, fresh=True)
    assert status['state'] == 'candidate' and status['ready'] is False
    assert status['guest'] == identity
    checked(builder, 'gzip -1 -c /output/job/candidate.ext4 > /output/candidate.gz', timeout=120)
    compressed = deployment.root / 'images' / 'rust.gz'
    with compressed.open('xb') as output:
        count = 0
        for block in builder.files.read('/output/candidate.gz', format='stream', user='root'):
            count += len(block)
            assert count <= 512 * 1024**2
            output.write(block)
        output.flush()
        os.fsync(output.fileno())
    disk = deployment.root / 'images' / 'rust.ext4'
    with gzip.open(compressed, 'rb') as incoming, disk.open('xb') as output:
        remaining = status['disk_bytes']
        while remaining:
            block = incoming.read(min(remaining, 1024**2))
            assert block
            write_sparse(output, block)
            remaining -= len(block)
        assert not incoming.read(1)
        output.truncate(status['disk_bytes'])
        output.flush()
        os.fsync(output.fileno())
    disk.chmod(0o400)
    assert file_hash(disk) == status['rootfs_sha256']
    deployment.close_builder(builder_id)
    intent = dict(id=secrets.token_hex(16), nonce=secrets.token_hex(16), rootfs_sha256=status['rootfs_sha256'])
    _save(deployment.root / 'inputs' / 'publication-intent.json', intent, fresh=True)
    candidate = stage(deployment.services.layout['registry'], kernel, disk, status, 1, 1, 512, 768,
        candidate_id=intent['id'], ownership={key: intent[key] for key in ('id', 'nonce')},
        registry_owner_uid=deployment.services.layout['center_identity'][0],
        rust_guest_profile=profile)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    url = deployment.services.sdk_url
    code, body = fetch(opener, url, 'POST', '/sandproof/template-publications', deployment.key,
                       {'candidateID': candidate})
    assert code in (200, 500, 503)
    publication = json.loads(body) if code == 200 else None
    deadline = time.monotonic() + 600
    while publication is None or publication.get('state') == 'checking':
        assert time.monotonic() < deadline, 'original publication outcome remains unknown'
        time.sleep(.2)
        code, body = fetch(opener, url, 'GET', '/sandproof/template-publications/' + candidate,
                           deployment.key, timeout=min(5, max(.001, deadline-time.monotonic())))
        assert code in (200, 404, 500, 503)
        publication = json.loads(body) if code == 200 else None
    assert publication['state'] == 'ready'
    deployment.services.wait_ready()
    return publication['template']['templateId']
