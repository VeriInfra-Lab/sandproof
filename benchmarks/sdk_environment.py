"""Shared bounded SDK transport; template preparation is a separate concern."""
from pathlib import Path, PurePosixPath
import shlex
import time

from e2b import FileType
from harbor.environments.capabilities import EnvironmentCapabilities
from harbor.environments.e2b import E2BEnvironment

from benchmarks.artifact_budget import (
    ArtifactBudget, DEFAULT_ARTIFACT_LIMITS, artifact_output, json_string_bytes,
    closing_artifact_stream, validate_destination,
)
from tools.benchmark_tasks import InvalidInput


class SdkEnvironment(E2BEnvironment):
    def __init__(self, *, sdk_options, artifact_limits=None, **kwargs):
        self.artifact_budget = ArtifactBudget(
            DEFAULT_ARTIFACT_LIMITS if artifact_limits is None else artifact_limits)
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
        budget = self.artifact_budget
        budget.entry()
        # Charge command JSON plus ample fixed space for metadata/errors before
        # dispatch. Rejected commands are not sent or silently replayed.
        budget.charge(json_string_bytes(command) + 1024)
        event = {'command': command, 'return_code': None}
        self.events.append(event)
        try:
            result = await super().exec(command, **kwargs)
            budget.charge(json_string_bytes(result.stdout) + json_string_bytes(result.stderr))
            event.update(return_code=result.return_code,
                         stdout=result.stdout, stderr=result.stderr)
            return result
        except BaseException as error:
            event['error_type'] = type(error).__name__[:128]
            raise
        finally:
            event['seconds'] = round(time.monotonic() - started, 3)

    def retain_text(self, target_path, content):
        """Account additional copies of command output in local diagnostics."""
        self.artifact_budget.entry()
        with artifact_output(self.trial_paths.trial_dir, target_path, self.artifact_budget) as output:
            for start in range(0, len(content), 16384):
                data = content[start:start + 16384].encode('utf-8')
                self.artifact_budget.charge(len(data))
                output.write(data)

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
        budget = self.artifact_budget
        budget.entry()
        file_bytes = 0
        try:
            with artifact_output(self.trial_paths.trial_dir, target_path, budget) as output:
                info = await self._sandbox.files.get_info(source_path)
                if info.type != FileType.FILE:
                    budget.reject('artifact_symlink_unsupported')
                reader = await self._sandbox.files.read(source_path, format='stream')
                async with closing_artifact_stream(reader, budget) as stream:
                    async for chunk in stream:
                        file_bytes += len(chunk)
                        if file_bytes > budget.limits['max_file_bytes']:
                            budget.reject('artifact_file_bytes_exceeded')
                        budget.charge(len(chunk))
                        output.write(chunk)
        except BaseException:
            # OracleAgent deliberately swallows download errors. Keep that
            # missing evidence visible to the runner, including cancellation.
            budget.note_failure('artifact_download_failed')
            raise

    async def download_dir(self, source_dir, target_dir):
        budget = self.artifact_budget

        async def visit(source, target, depth):
            budget.entry(depth)
            info = await self._sandbox.files.get_info(str(source))
            if info.type != FileType.DIR:
                budget.reject('artifact_symlink_unsupported')
            seen = set()
            for entry in await self._sandbox.files.list(str(source)):
                path = PurePosixPath(entry.path)
                if (not path.is_absolute() or '..' in path.parts
                        or path.parent != source or path.name in ('.', '..')
                        or str(path) != entry.path or path.name in seen):
                    budget.reject('unsafe_or_duplicate_artifact_path')
                seen.add(path.name)
                if depth + 1 > budget.limits['max_depth']:
                    budget.reject('artifact_depth_exceeded')
                if entry.type == FileType.FILE:
                    await self.download_file(str(path), target / path.name)
                elif entry.type == FileType.DIR:
                    await visit(path, target / path.name, depth + 1)
                else:
                    budget.reject('artifact_symlink_unsupported')

        source = PurePosixPath(source_dir)
        if not source.is_absolute() or '..' in source.parts:
            budget.reject('unsafe_artifact_source')
        try:
            validate_destination(self.trial_paths.trial_dir, target_dir, budget, directory=True)
            await visit(source, Path(target_dir), 0)
        except BaseException:
            budget.note_failure('artifact_download_failed')
            raise

    async def download_dir_filtered(self, **kwargs):
        # Upstream uses an unbudgeted host temporary tar and extracts it.
        self.artifact_budget.reject('filtered_artifact_download_unsupported')
