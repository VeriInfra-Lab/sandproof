"""Private test-only handoff of one VM's original native /init material.

The original host audit maps the public identity. No secret search, public
gateway token substitution, startup replay, or management credential export.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import hashlib
import json
from pathlib import Path
import re
import shlex
import uuid

from e2b import Sandbox
from sdk_server_smoke import options
from tools.check_kvm_host import unique_keys
from tools.guest_artifacts import fingerprint
from tools.system_acceptance import private_path, write_once

LIMIT = 64 * 1024


def original_init(path, guest, template):
    _, raw = fingerprint(private_path(path), LIMIT, content=True)
    secret = json.loads(raw, object_pairs_hook=unique_keys)
    if set(secret) - {'token', 'sha512', 'env_vars', 'template'}:
        raise ValueError('unexpected native guest material')
    token = secret['token']
    if (type(token) is not str or re.fullmatch('[A-Za-z0-9_-]{32,128}', token) is None
            or hashlib.sha512(token.encode()).hexdigest() != secret['sha512']):
        raise ValueError('native guest credential integrity')
    binding = secret['template']
    if binding['guest'] != guest or binding['template_id'] != template:
        raise ValueError('original guest/template identity mismatch')
    # Match sandbox-node/src/startup.rs::init_payload, including create overrides.
    startup = binding.get('startup')
    env = dict(startup['env_vars']) if startup is not None else {}
    env.update(secret.get('env_vars', {}))
    payload = dict(accessToken=token, defaultUser=startup['user'] if startup else 'root', envVars=env)
    if startup is not None:
        payload['defaultWorkdir'] = startup['workdir']
    encoded = json.dumps(payload)
    if len(encoded.encode()) > LIMIT:
        raise ValueError('original init payload exceeds private fixture bound')
    return encoded


def prepare(deployment, root, template, guest):
    owner = options(deployment.key, deployment.services.sdk_url, deployment.services.sdk_url)
    vm = Sandbox.create(template=template, timeout=300, allow_internet_access=False, **owner)
    # Persist the identity before any other operation. The original outer
    # deployment cleanup retains responsibility even if preparation is unknown.
    write_once(root/'restart-created.json', dict(sandbox_id=vm.sandbox_id))
    native = deployment.native_id(vm.sandbox_id, index=0)
    if type(native) is not str or re.fullmatch('sb-[a-z0-9]+-[0-9]+', native) is None:
        raise ValueError('original audited native identity required')
    payload = original_init(Path(deployment.settings[0]['node_state_dir'])/'secrets'/native,
                            guest, template)
    deployment.services.wait_ready()
    directory = '/var/tmp/envd-restart-' + uuid.uuid4().hex
    vm.commands.run('mkdir -p -- /var/tmp && mkdir -m 0700 -- ' + shlex.quote(directory),
                    user='root', timeout=15)
    path = directory + '/init.json'
    vm.files.write(path, payload, user='root')
    vm.commands.run('chmod 0600 -- ' + shlex.quote(path), user='root', timeout=15)
    receipt = dict(sandbox_id=vm.sandbox_id, template=template, guest=guest, init_path=path)
    handoff = root/'restart-fixture.json'
    write_once(handoff, receipt)
    return handoff


def load(path, template, guest):
    _, raw = fingerprint(private_path(path), LIMIT, content=True)
    value = json.loads(raw, object_pairs_hook=unique_keys)
    if (set(value) != {'sandbox_id', 'template', 'guest', 'init_path'}
            or value['template'] != template or value['guest'] != guest
            or type(value['sandbox_id']) is not str or not 0 < len(value['sandbox_id']) <= 256
            or type(value['init_path']) is not str
            or re.fullmatch('/var/tmp/envd-restart-[0-9a-f]{32}/init.json', value['init_path']) is None):
        raise ValueError('exact protected restart fixture required')
    return value
