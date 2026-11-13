"""Run an approved static template without dispatching a build."""
import re

from benchmarks.generic_environment import GenericEnvironment


class PreparedTemplateEnvironment(GenericEnvironment):
    source_kind = 'prepared-ci'
    internet_access = False

    def __init__(self, *, fixture, **kwargs):
        self.fixture = fixture
        self.guest_preflight = 'not_run'
        super().__init__(**kwargs)
        self.source_kind = fixture.source_kind
        self.internet_access = self.source_kind == 'prepared-original'

    async def prepare_template(self):
        # No public build request, host download or prior execution is reused.
        # Registration is owned by the trusted enclosing KVM harness.
        descriptor = self.fixture.template_descriptor()
        # This labels the immutable prepared binding for the existing private
        # create-reconciliation protocol. No public build is dispatched.
        prefix = 'prebuilt-' if self.source_kind == 'prepared-original' else 'ci-'
        if re.fullmatch(prefix + r'[0-9a-f]{32}', descriptor.get('template_id', '')) is None:
            raise ValueError('prepared runtime requires its explicit static template binding')
        self.build_id = descriptor['template_id'].removeprefix(prefix)
        resources = descriptor['resources']
        self.template = {
            'templateId': descriptor['template_id'], 'revision': descriptor['revision'],
            'cpuCount': resources['cpu_count'], 'memoryMib': resources['memory_mib'],
            'diskBytes': resources['disk_bytes'], 'hostMemoryBytes': resources['host_memory_bytes'],
            'kernelSha256': descriptor['kernel']['sha256'],
            'rootfsSha256': descriptor['rootfs']['sha256'],
            'startup': self.fixture.startup,
        }
        self.checkpoint(self.source_kind + '-fixture.json', self.fixture.evidence())

    async def start(self, force_build=False):
        if force_build:
            raise ValueError('prepared templates do not dispatch builds')
        self.guest_preflight = 'not_run'
        await super().start(force_build=False)
        self.guest_preflight = 'running'
        try:
            boundary = await self.exec(self.fixture.guest_preflight_command(), cwd='/', user='root', timeout_sec=30)
        except BaseException:
            self.guest_preflight = 'failed'
            raise
        if boundary.return_code:
            self.guest_preflight = 'failed'
            raise ValueError('prepared guest prerequisites or clean boundary rejected')
        self.guest_preflight = 'passed'
        self.checkpoint(self.source_kind + '-fixture.json', self.fixture.evidence() | {'guest_preflight': 'passed'})
