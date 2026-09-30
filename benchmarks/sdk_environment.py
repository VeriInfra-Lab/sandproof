"""Shared bounded SDK transport; template preparation is a separate concern."""
from pathlib import Path, PurePosixPath
import shlex
import time

from e2b import FileType
from harbor.environments.capabilities import EnvironmentCapabilities
from harbor.environments.e2b import E2BEnvironment

from tools.benchmark_tasks import InvalidInput


class SdkEnvironment(E2BEnvironment):
    def __init__(self, *, sdk_options, **kwargs):
        self.sdk_options = sdk_options
        self.events = []
        self.sandbox_id = None
        self.kill_confirmed = False  # SDK acknowledgment, NOT exact host reclaim.
        self.create_dispatched = False
        super().__init__(**kwargs)

    @property
    def capabilities(self):
        return EnvironmentCapabilities()

    async def stop(self, delete=True):
        if self._sandbox is not None:
            self.kill_confirmed = await self._sandbox.kill()
            if not self.kill_confirmed:
                raise RuntimeError('sandbox deletion was not acknowledged')
            self._sandbox = None

    async def exec(self, command, **kwargs):
        started = time.monotonic()
        event = {'command': command, 'return_code': None}
        self.events.append(event)
        try:
            result = await super().exec(command, **kwargs)
            event.update(return_code=result.return_code,
                         stdout=result.stdout, stderr=result.stderr)
            return result
        except BaseException as error:
            event['error_type'] = type(error).__name__
            raise
        finally:
            event['seconds'] = round(time.monotonic() - started, 3)

    async def upload_file(self, source_path, target_path):
        source = Path(source_path)
        if source.is_symlink() or not source.is_file():
            raise InvalidInput('only verified regular input files may be uploaded')
        await self._sandbox.files.write(target_path, source.read_bytes())
        mode = '755' if source.stat().st_mode & 0o111 else '644'
        result = await self.exec('chmod ' + mode + ' -- ' + shlex.quote(target_path), cwd='/', timeout_sec=30)
        if result.return_code:
            raise RuntimeError('input mode restoration failed')

    async def upload_dir(self, source_dir, target_dir):
        source = Path(source_dir)
        await self._sandbox.files.make_dir(target_dir)
        for path in sorted(source.rglob('*')):
            if path.is_symlink():
                raise InvalidInput('input symlink is unsupported')
            target = str(PurePosixPath(target_dir) / path.relative_to(source).as_posix())
            if path.is_dir():
                await self._sandbox.files.make_dir(target)
            elif path.is_file():
                await self.upload_file(path, target)

    async def download_file(self, source_path, target_path):
        target = Path(target_path)
        root = self.trial_paths.trial_dir.resolve()
        if target.is_symlink() or not target.resolve().is_relative_to(root):
            raise InvalidInput('guest log destination escapes the trial directory')
        data = await self._sandbox.files.read(source_path, format='bytes')
        if len(data) > 96 * 1024**2:
            raise InvalidInput('guest log exceeds download budget')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)

    async def download_dir(self, source_dir, target_dir):
        source = PurePosixPath(source_dir)
        for entry in await self._sandbox.files.list(source_dir):
            path = PurePosixPath(entry.path)
            if path.parent != source or path.name in ('.', '..'):
                raise InvalidInput('guest returned an unexpected log path')
            target = Path(target_dir) / path.name
            if entry.type == FileType.FILE:
                await self.download_file(str(path), target)
            elif entry.type == FileType.DIR:
                await self.download_dir(str(path), target)
            else:
                raise InvalidInput('guest log symlinks are unsupported')
