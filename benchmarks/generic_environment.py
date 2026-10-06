"""Original environment -> self-service template -> original E2B SDK.

Explicit reviewed environment variants are separately recorded; no host execution.
Only template preparation is replaced; Harbor owns the oracle/verifier phases.
"""
import asyncio
import hashlib
import io
import json
import math
from pathlib import Path, PurePosixPath
import re
import secrets
import tarfile
import time

from e2b import AsyncSandbox

from benchmarks.sdk_environment import SdkEnvironment
from benchmarks.environment_repairs import PRUNED_SOURCE, archive_repaired_context, repair_profile
from tools.benchmark_tasks import InvalidInput
from tools.template_build_client import BuildClient, request_window

MIB = 1024 * 1024
BUILD_LOG_BYTES = 2 * MIB
BUILD_LOG_PAGE = 65536


def archive_context(entry, source, output):
    """Package ONLY locked environment files, preserving exact bytes/modes."""
    root = (Path(source) / entry['source_path'] / 'environment').resolve(strict=True)
    files = [f for f in entry['files'] if f['role'] == 'environment']
    expected = {f['path'].removeprefix('environment/') for f in files}
    actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if not p.is_dir()}
    if actual != expected or 'Dockerfile' not in expected:
        raise InvalidInput('environment context differs from the locked tree')
    with tarfile.open(output, 'x', format=tarfile.PAX_FORMAT) as archive:
        for spec in sorted(files, key=lambda f: f['path']):
            relative = PurePosixPath(spec['path'])
            if relative.parts[0] != 'environment' or '..' in relative.parts or relative.is_absolute():
                raise InvalidInput('private or escaping file in environment context')
            relative = PurePosixPath(*relative.parts[1:])
            path = root / relative
            if (path.is_symlink() or not path.is_file()
                    or not path.resolve(strict=True).is_relative_to(root)):
                raise InvalidInput('only regular context files are supported')
            data = path.read_bytes()
            mode = 0o755 if spec['mode'] == '100755' else 0o644
            if (len(data) != spec['bytes'] or hashlib.sha256(data).hexdigest() != spec['sha256']
                    or bool(path.stat().st_mode & 0o111) != (mode == 0o755)):
                raise InvalidInput('context changed after source validation')
            member = tarfile.TarInfo(relative.as_posix())
            member.size, member.mode, member.mtime = len(data), mode, 0
            archive.addfile(member, io.BytesIO(data))
    return hashlib.sha256(Path(output).read_bytes()).hexdigest()


def build_request(entry, config, source_kind):
    requirements = entry['requirements']
    values = [requirements[k] for k in ('cpus', 'memory_mib', 'disk_mib')]
    if any(type(v) is not int or v <= 0 for v in values):
        raise InvalidInput('explicit positive CPU/memory/total-disk shape required')
    timeout = requirements['build_timeout_sec']
    if not math.isfinite(timeout) or timeout < 1:
        raise InvalidInput('invalid original build deadline')
    if (config.os.value != 'linux' or config.gpus or config.tpu or config.mcp_servers
            or config.network_mode.value != 'public' or config.allowed_hosts):
        raise InvalidInput('unsupported OS/device/service/network requirements')
    request = {'kind': 'dockerfile' if source_kind == PRUNED_SOURCE else source_kind,
               'timeoutSeconds': int(timeout), 'allowInternetAccess': True,
               'resources': dict(zip(('cpuCount', 'memoryMiB', 'diskMiB'), values), minFreeMiB=64)}
    if source_kind == 'oci':
        if not config.docker_image or config.docker_image != entry['upstream_image_reference']:
            raise InvalidInput('original public image reference required')
        request['image'] = config.docker_image
    elif source_kind == PRUNED_SOURCE:
        profile = repair_profile(entry)
        if config.docker_image != profile['image']:
            raise InvalidInput('registered original image required for environment repair')
    elif source_kind != 'dockerfile':
        raise InvalidInput('unknown environment source kind')
    return request


def validate_template(template, entry):
    req = entry['requirements']
    if (not isinstance(template, dict)
            or (template.get('cpuCount'), template.get('memoryMib'), template.get('diskBytes'))
               != (req['cpus'], req['memory_mib'], req['disk_mib'] * MIB)
            or not re.fullmatch(r'tpl-[0-9a-f]{32}', template.get('templateId', ''))
            or not template.get('revision')
            or not re.fullmatch(r'sha256:[0-9a-f]{64}', template.get('imageDigest', ''))
            or any(not re.fullmatch(r'[0-9a-f]{64}', template.get(k, '')) for k in ('kernelSha256', 'rootfsSha256'))
            or not isinstance(template.get('startup'), dict)):
        raise InvalidInput('published template identity/resources/defaults mismatch')
    return template


class GenericEnvironment(SdkEnvironment):
    def __init__(self, *, entry, source, source_kind, build_url, sdk_options, start_audit=None, **kwargs):
        if 'build_mode' in kwargs:
            raise InvalidInput("build mode selection is no longer supported")
        self.entry, self.source, self.source_kind = entry, source, source_kind
        self.client = BuildClient(build_url, sdk_options['api_key'])
        self.build_result = None
        self.build_log_error = None
        self.build_id = None
        self.build_dispatched = False
        self.submission_key = secrets.token_hex(16)
        self.template = None
        self.runtime_intent = None
        self.runtime_attempt = None
        self.start_audit = start_audit
        self.environment_repair = None
        super().__init__(sdk_options=sdk_options, **kwargs)

    async def prepare_template(self):
        self.environment_repair = None
        request = build_request(self.entry, self.task_env_config, self.source_kind)
        context = self.trial_paths.trial_dir / 'context.tar'
        saved_path = self.trial_paths.trial_dir / 'build-intent.json'
        saved = None
        if saved_path.exists() or saved_path.is_symlink():
            if saved_path.is_symlink() or saved_path.stat().st_size > 65536:
                raise InvalidInput('invalid original build intent')
            saved = json.loads(saved_path.read_text())
        if self.source_kind in ('dockerfile', PRUNED_SOURCE) and saved is not None:
            if context.is_symlink() or not context.is_file():
                raise InvalidInput('original build context is unavailable')
            digest = hashlib.sha256()
            with context.open('rb') as incoming:
                while block := incoming.read(MIB):
                    digest.update(block)
            context_hash = digest.hexdigest()
            if context_hash != saved.get('context_sha256'):
                raise InvalidInput('original build context changed')
            if self.source_kind == PRUNED_SOURCE:
                self.environment_repair = saved.get('environment_repair')
                if not isinstance(self.environment_repair, dict) or self.environment_repair.get('context_sha256') != context_hash:
                    raise InvalidInput('original repaired context provenance changed')
        elif self.source_kind == 'dockerfile':
            context_hash = archive_context(self.entry, self.source, context)
        elif self.source_kind == PRUNED_SOURCE:
            self.environment_repair = archive_repaired_context(self.entry, context)
            context_hash = self.environment_repair['context_sha256']
        else:
            context_hash = None
        intent = self.client.intent_binding() | {'build_mode': 'windowed', 'request': request, 'context_sha256': context_hash,
                  'task_files_sha256': self.entry['files_sha256'], 'source_kind': self.source_kind,
                  'original_environment': self.source_kind != PRUNED_SOURCE,
                  'environment_repair': self.environment_repair}
        if saved is not None:
            if any(saved.get(key) != value for key, value in intent.items()):
                raise InvalidInput('original build intent conflicts; do not replace uncertain work')
            window, self.submission_key = saved.get('requestWindow'), saved.get('idempotency_key')
            if not isinstance(self.submission_key, str) or not self.submission_key:
                raise InvalidInput('original build intent lacks window/key')
        else:
            opened = await asyncio.to_thread(self.client.request, 'POST', '/sandproof/template-build-windows',
                                             {'durationMs': request['timeoutSeconds'] * 1000})
            window = opened.get('requestWindow')
        try:
            window = request_window(window)
        except ValueError as error:
            raise InvalidInput(str(error)) from error
        if saved is None:
            intent.update(idempotency_key=self.submission_key, requestWindow=window)
            # This checkpoint is fsynced before POST. After an uncertain response,
            # retry only this original namespace/window/key and canonical input.
            self.checkpoint('build-intent.json', intent)
        self.build_dispatched = True
        result = await asyncio.to_thread(self.client.request, 'POST', '/sandproof/template-builds',
                                         request | {'requestWindow': window}, token=self.submission_key)
        self.build_id = result['buildID']
        self.build_result = result
        if not re.fullmatch(r'[0-9a-f]{32}', self.build_id):
            raise InvalidInput('invalid build identity')
        self.checkpoint('build-status.json', result)
        path = '/sandproof/template-builds/' + self.build_id
        if result['state'] == 'AwaitingInput':
            if request['kind'] != 'dockerfile':
                raise InvalidInput('image import unexpectedly requires context')
            def upload():
                with context.open('rb') as incoming:
                    return self.client.request('PUT', path + '/context', incoming, size=context.stat().st_size)
            await asyncio.to_thread(upload)
        deadline = time.monotonic() + request['timeoutSeconds']
        try:
            while time.monotonic() < deadline:
                result = await asyncio.to_thread(self.client.request, 'GET', path)
                self.build_result = result
                self.checkpoint('build-status.json', result)
                if result['state'] == 'Ready':
                    self.template = validate_template(result['template'], self.entry)
                    return
                if result['state'] in ('Failed', 'CleanupUnknown'):
                    raise RuntimeError('template build: ' + str(result['failure']))
                await asyncio.sleep(1)
            raise TimeoutError('original build deadline; observe original build, do not replace it')
        finally:
            # Bounded best-effort diagnostics, not a cleanup or success witness.
            try:
                await self.capture_build_log(path)
            except Exception as error:
                # Keep the original build outcome and retain only the error
                # class: exception text may include request credentials.
                self.build_log_error = type(error).__name__

    async def capture_build_log(self, path):
        log = {'offset': 0, 'nextOffset': 0, 'text': '', 'complete': False, 'truncated': False}
        try:
            async with asyncio.timeout(30):
                while True:
                    remaining = BUILD_LOG_BYTES - log['nextOffset']
                    # At the exact cap, one byte distinguishes EOF from truncation.
                    limit = min(BUILD_LOG_PAGE, remaining) if remaining else 1
                    page = await asyncio.to_thread(self.client.request, 'GET',
                        path + f'/logs?offset={log["nextOffset"]}&limit={limit}')
                    if (not isinstance(page, dict) or type(page.get('offset')) is not int
                            or page['offset'] != log['nextOffset']
                            or type(page.get('nextOffset')) is not int
                            or not 0 <= page['nextOffset'] - page['offset'] <= limit
                            or not isinstance(page.get('text'), str)):
                        raise InvalidInput('invalid build log page')
                    size = page['nextOffset'] - page['offset']
                    # Offsets count raw bytes, not Unicode characters/re-encoding.
                    if (len(page['text'].encode('utf-8')) > 3 * size
                            or bool(page['text']) != bool(size)):
                        raise InvalidInput('invalid build log text')
                    if size > remaining:
                        log['truncated'] = True
                        break
                    log['nextOffset'] = page['nextOffset']
                    log['text'] += page['text']
                    if size < limit:
                        log['complete'] = self.build_result['state'] in ('Ready', 'Failed')
                        break
        finally:
            # Retain partial pages even if a later request fails. JSON expansion
            # and this additional copy count against the trial evidence budget.
            self.retain_text(self.trial_paths.trial_dir / 'build-log.json',
                             json.dumps(log, indent=2) + '\n')

    def checkpoint(self, name, value):
        path = self.trial_paths.trial_dir / name
        with path.open('w') as output:
            json.dump(value, output, indent=2)
            output.flush()
            import os
            os.fsync(output.fileno())
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)

    async def start(self, force_build=False):
        if not self._network_is_public:
            raise InvalidInput('only the fixed public IPv4 policy is supported in this runner')
        deadline = time.monotonic() + self.entry['requirements']['build_timeout_sec']
        await self.prepare_template()
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('original initialization deadline before runtime create')
        startup = self.template['startup']
        self._workdir = startup['workdir']
        self.default_user = startup['user']
        self.runtime_attempt = secrets.token_hex(16)
        metadata = {'benchmark': 'terminal-bench-2.0', 'task': self.environment_name,
                    'build': self.build_id, 'revision': self.template['revision'],
                    'runtime_attempt': self.runtime_attempt}
        # This is a correlation identity, not an idempotency key. Never replay
        # create after a lost response. The local operator can find Starting
        # records in its private journal; the public list only shows Running.
        self.runtime_intent = {'metadata': metadata, 'template': self.template}
        self.checkpoint('runtime-intent.json', self.runtime_intent)
        self.create_dispatched = True
        self._sandbox = await AsyncSandbox.create(
            template=self.template['templateId'], timeout=86400, allow_internet_access=True,
            metadata=metadata,
            envs=self._startup_env(), **(self.sdk_options | {'request_timeout': remaining}))
        self.sandbox_id = self._sandbox.sandbox_id
        self.checkpoint('runtime-identity.json', {'sandbox_id': self.sandbox_id, 'template': self.template})
        info = await self._sandbox.get_info()
        if (info.template_id, info.cpu_count, info.memory_mb) != (
                self.template['templateId'], self.entry['requirements']['cpus'], self.entry['requirements']['memory_mib']):
            raise InvalidInput('actual SDK identity/resource profile differs from task')
        if self.start_audit is not None:
            self.checkpoint('host-shape.json', await asyncio.to_thread(self.start_audit, self))
        boundary = await self.exec('test ! -e /solution && test ! -e /tests', cwd='/', user='root', timeout_sec=30)
        if boundary.return_code:
            raise InvalidInput('template leaked reserved phase directories')
        await self.ensure_dirs(['/logs/agent', '/logs/verifier', '/logs/artifacts'])
        await self._upload_environment_dir_after_start()
