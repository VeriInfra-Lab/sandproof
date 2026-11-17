"""Bounded read-only E16 samples of original services, recorded VMs and volume.

No process discovery, signals, cgroup writes, physical completion or quota
verdict. Missing samples and unobserved process lifetime edges remain explicit.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import stat
import threading
import time

from _center_host_services import process_facts
from lifecycle_unload_runtime import original_process
from server_bounded_churn import live_identity
from server_process_crash import process_start_ticks
from tools import guest_probe, service_cgroup
from tools.service_storage_observation import used_volume_bytes
from tools.system_acceptance import private_path

MAX_SAMPLES = 8192
MAX_EPOCHS = 64
MAX_BYTES = 64 * 1024**2
U64 = 2**64


def options(args):
    enabled = getattr(args, 'measure_resources', False)
    interval = getattr(args, 'resource_sample_interval_ms', 500)
    if (type(enabled) is not bool or type(interval) is not int
            or not 500 <= interval <= 5000):
        raise ValueError('explicit sampling flag and 500..5000ms interval required')
    return enabled, interval


def integer(value):
    if type(value) is not int or not 0 <= value < U64:
        raise ValueError('bounded nonnegative original resource counter required')
    return value


def boot_id():
    return Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def stat_fields(raw, pid):
    prefix, tail = raw.rsplit(b')', 1)
    fields = tail.split()
    if (int(prefix.split(b'(', 1)[0]) != pid or len(fields) < 20
            or fields[0] in (b'Z', b'X', b'x')):
        raise ProcessLookupError('original process no longer live')
    return dict(start_time_ticks=integer(int(fields[19])),
        cpu_ticks=integer(int(fields[11])+int(fields[12])))


def process_sample(identity):
    """Pinned procfs directory; RSS from smaps_rollup, not statm/VmRSS."""
    pid = identity['pid']
    fd = os.open(Path('/proc')/str(pid), os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        before = stat_fields(guest_probe.proc_snapshot('stat', fd), pid)
        if before['start_time_ticks'] != identity['start_time_ticks']:
            raise ValueError('original PID/start identity changed')
        status = guest_probe.proc_values(guest_probe.proc_snapshot('status', fd),
            {b'Pid':1, b'Tgid':1, b'VmHWM':1024})
        memory = guest_probe.proc_values(guest_probe.proc_snapshot('smaps_rollup', fd), {b'Rss':1024})
        after = stat_fields(guest_probe.proc_snapshot('stat', fd), pid)
        if (status[b'Pid'] != pid or status[b'Tgid'] != pid
                or after['start_time_ticks'] != before['start_time_ticks']
                or after['cpu_ticks'] < before['cpu_ticks']):
            raise ValueError('original process changed while measuring')
        return dict(cpu_ticks=after['cpu_ticks'], rss_bytes=integer(memory[b'Rss']),
            kernel_process_peak_rss_bytes=integer(status[b'VmHWM']))
    finally:
        os.close(fd)


def group_sample(view):
    cpu = view['cpu_stat']
    return dict(cpu_usage_usec=integer(cpu['usage_usec']),
        memory_current_bytes=integer(view['memory_current']),
        kernel_group_peak_memory_bytes=integer(view['memory_peak']))


class LifecycleResources:
    def __init__(self, runtime, *, phase, interval_ms, deadline):
        self.runtime, self.phase = runtime, phase
        self.interval_ms, self.deadline = interval_ms, deadline
        self.candidate = runtime.services.manifest['candidate']
        if re.fullmatch('[0-9a-f]{40}',self.candidate) is None:
            raise ValueError('exact original prepared candidate required')
        self.run_id = secrets.token_hex(16)  # Evidence correlation only; never a business identity/permission.
        self.bindings = copy.deepcopy(runtime.services.processes.bindings)
        if not 3 <= len(self.bindings) <= 4:
            raise ValueError('one original finite service cohort required')
        self.boot = boot_id()
        self.ticks = os.sysconf('SC_CLK_TCK')
        if self.ticks <= 0:
            raise ValueError('original positive CPU clock tick rate required')
        self.epochs, self.handles, self.vms = [], {}, {}
        if len(runtime.services.native_pools) != 1:
            raise ValueError('one original lifecycle VM pool required')
        self.native = copy.deepcopy(runtime.services.native_pools[0])
        self.group = service_cgroup.observe(runtime.services.processes.record)
        self.volume_path = private_path(runtime.quota['mount'], directory=True)
        self.volume_fd = os.open(self.volume_path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        m = os.fstat(self.volume_fd)
        self.volume_identity = (m.st_dev, m.st_ino)
        self.volume = dict(samples=0, unknown_samples=0, baseline_used_bytes=None,
            sampled_peak_used_bytes=None, final_used_bytes=None)
        root = private_path(runtime.root, directory=True)
        self.path = root/'lifecycle-resource-samples.jsonl'
        try:
            self.fd = os.open(self.path, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        except BaseException:
            os.close(self.volume_fd)
            raise
        meta = os.fstat(self.fd)
        self.raw_identity = (meta.st_dev, meta.st_ino)
        self.raw_digest = None
        self.bytes, self.samples, self.unknown = 0, 0, 0
        self.cpu_ns = self.wall_ns = self.max_wall_ns = self.max_gap_ns = 0
        self.last_time = self.failure = None
        self.closed = False
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self._run, name='lifecycle-resource-sampler', daemon=True)

    def _epoch(self, kind, role, identity, binding=None):
        if len(self.epochs) >= MAX_EPOCHS:
            raise ValueError('finite process epoch capacity exhausted')
        row = dict(index=len(self.epochs), kind=kind, role=role,
            identity=dict(copy.deepcopy(identity), boot_id=self.boot), binding=copy.deepcopy(binding),
            samples=0, unknown_samples=0, inactive_samples=0, first=None, last=None,
            sampled_peak_rss_bytes=None, kernel_process_peak_rss_bytes=None)
        self.epochs.append(row)
        return row

    def _metrics(self, epoch, value, now):
        value = dict(value, at_monotonic_ns=now)
        if epoch['last'] is not None and value['cpu_ticks'] < epoch['last']['cpu_ticks']:
            raise ValueError('original process CPU counter regressed')
        epoch['samples'] += 1
        if epoch['first'] is None:
            epoch['first'] = copy.deepcopy(value)
        epoch['last'] = copy.deepcopy(value)
        for source, target in (('rss_bytes', 'sampled_peak_rss_bytes'),
                ('kernel_process_peak_rss_bytes', 'kernel_process_peak_rss_bytes')):
            prior = epoch[target]
            epoch[target] = value[source] if prior is None else max(prior, value[source])
        return dict(epoch=epoch['index'], identity=copy.deepcopy(epoch['identity']), state='observed', **value)

    def _services(self, now):
        processes = self.runtime.services.processes
        if processes.bindings != self.bindings:
            raise ValueError('original service cohort configuration changed')
        rows = []
        for name, role in self.bindings.items():
            child = processes.children.get(name)
            if child is None:
                self.unknown += 1
                rows.append(dict(role=name, state='not_started'))
                continue
            epoch = self.handles.get(child)
            if child.poll() is not None:
                if epoch is not None: epoch['inactive_samples'] += 1
                rows.append(dict(role=name, epoch=None if epoch is None else epoch['index'], state='not_live'))
                continue
            argv = [str(role['binary']), str(role['config'])] if name == 'redis' else None
            try:
                facts = process_facts(child, role['binary'], role['config'], processes.record,
                    uid=role['uid'], gid=role['gid'], argv=argv)
                if epoch is None:
                    epoch = self._epoch('service', name, facts)
                    self.handles[child] = epoch  # Retain the exact original Popen, never just a PID.
                if dict(facts, boot_id=self.boot) != epoch['identity']:
                    raise ValueError('an original service epoch changed identity')
                value = process_sample(facts)
                if process_facts(child, role['binary'], role['config'], processes.record,
                        uid=role['uid'], gid=role['gid'], argv=argv) != facts:
                    raise ValueError('original service changed during resource sample')
                rows.append(self._metrics(epoch, value, now))
            except (OSError, RuntimeError):
                self.unknown += 1
                if epoch is not None: epoch['unknown_samples'] += 1
                rows.append(dict(role=name, epoch=None if epoch is None else epoch['index'], state='unknown'))
            except ValueError:
                if child.poll() is not None:
                    # process_facts also rejects a child that exits during
                    # its final identity check. Preserve the gap, never adopt
                    # another PID or turn that incomplete read into zero.
                    self.unknown += 1
                    if epoch is not None: epoch['unknown_samples'] += 1
                    rows.append(dict(role=name, epoch=None if epoch is None else epoch['index'],
                        state='exited_during_sample'))
                    continue
                if epoch is not None: raise
                # A newly launched wrapper has not necessarily exec'd/dropped
                # privileges yet. It does not create an accepted identity.
                self.unknown += 1
                rows.append(dict(role=name, state='not_ready'))
        return rows

    def _vm_samples(self, now):
        if self.runtime.services.native_pools[0] != self.native:
            raise ValueError('original VM pool configuration changed')
        for session, run in copy.deepcopy(self.runtime.runs.copy()).items():
            source = run['source']; key = (source['pid'], source['start_time_ticks'], source['boot_id'])
            if key not in self.vms:
                epoch = self._epoch('vm', 'vm-'+str(len(self.vms)),
                    dict(pid=source['pid'], start_time_ticks=source['start_time_ticks']),
                    dict(session=session, context=run['context']))
                if source['boot_id'] != self.boot:
                    raise ValueError('original recorded VM belongs to another boot')
                self.vms[key] = (epoch, copy.deepcopy(run))
            elif any(self.vms[key][1][field] != run[field] for field in ('source', 'context')):
                raise ValueError('recorded original VM facts changed')
        rows = []
        for epoch, run in self.vms.values():
            source = run['source']
            if not original_process(source):
                epoch['inactive_samples'] += 1
                rows.append(dict(epoch=epoch['index'], state='not_live'))
                continue
            try:
                before = live_identity(self.native, run['context'], process_start_ticks)
                if (before['pid'], before['start_time_ticks']) != (source['pid'], source['start_time_ticks']):
                    raise ValueError('original VM identity changed')
                value = process_sample(epoch['identity'])
                after = live_identity(self.native, run['context'], process_start_ticks)
                if before != after:
                    raise ValueError('original VM resources changed during sampling')
                rows.append(self._metrics(epoch, value, now))
            except OSError:
                self.unknown += 1; epoch['unknown_samples'] += 1
                rows.append(dict(epoch=epoch['index'], state='unknown'))
            except (ValueError, AssertionError):
                if original_process(source): raise
                self.unknown += 1; epoch['unknown_samples'] += 1
                rows.append(dict(epoch=epoch['index'], state='exited_during_sample'))
        return rows

    def _volume(self):
        try:
            current = self.volume_path.lstat()
            if not stat.S_ISDIR(current.st_mode) or (current.st_dev, current.st_ino) != self.volume_identity:
                raise ValueError('original measured volume directory replaced')
            used = integer(used_volume_bytes(self.volume_fd))
        except OSError:
            self.volume['unknown_samples'] += 1
            self.volume['final_used_bytes'] = None
            self.unknown += 1
            return None
        row = self.volume
        row['samples'] += 1
        if row['baseline_used_bytes'] is None: row['baseline_used_bytes'] = used
        peak = row['sampled_peak_used_bytes']
        row['sampled_peak_used_bytes'] = used if peak is None else max(peak, used)
        row['final_used_bytes'] = used
        return used

    def sample(self):
        started, cpu = time.monotonic_ns(), time.thread_time_ns()
        try:
            if self.samples >= MAX_SAMPLES or time.monotonic() > self.deadline:
                raise TimeoutError('original finite measurement capacity/deadline exhausted')
            if boot_id() != self.boot:
                raise ValueError('original boot identity changed')
            group = service_cgroup.observe(self.runtime.services.processes.record)
            if any(group[k] != self.group[k] for k in ('identity', 'path', 'limits')):
                raise ValueError('original service cgroup changed')
            group_values = group_sample(group)
            if any(group_values[key] < group_sample(self.group)[key]
                    for key in ('cpu_usage_usec', 'kernel_group_peak_memory_bytes')):
                raise ValueError('original service cgroup CPU counter regressed')
            row = dict(candidate=self.candidate, run_id=self.run_id,
                sequence=self.samples, at_monotonic_ns=started, phase=self.phase(),
                services=self._services(started), vms=self._vm_samples(started),
                service_group=group_values, service_group_identity=group['identity'],
                volume_used_bytes=self._volume(), volume_identity=list(self.volume_identity))
            if type(row['phase']) is not str or re.fullmatch('[a-z0-9-]{1,128}', row['phase']) is None:
                raise ValueError('bounded original lifecycle phase name required')
            self.group = group
            encoded = (json.dumps(row, sort_keys=True, separators=(',', ':'))+'\n').encode()
            if len(encoded) > 16384 or self.bytes+len(encoded) > MAX_BYTES:
                raise ValueError('finite original resource evidence byte capacity exhausted')
            view = memoryview(encoded)
            while view:
                written = os.write(self.fd, view)
                if written <= 0: raise OSError('resource evidence short write')
                self.bytes += written
                view = view[written:]
            self.samples += 1
            if self.last_time is not None: self.max_gap_ns = max(self.max_gap_ns, started-self.last_time)
            self.last_time = started
            if not hasattr(self, 'group_first'): self.group_first = copy.deepcopy(group_values)
            self.group_last = group_values
        finally:
            elapsed = time.monotonic_ns()-started
            self.cpu_ns += time.thread_time_ns()-cpu
            self.wall_ns += elapsed; self.max_wall_ns = max(self.max_wall_ns, elapsed)

    def _run(self):
        while not self.stop.wait(self.interval_ms/1000):
            try: self.sample()
            except Exception as error:
                self.failure = type(error).__name__
                break

    def start(self):
        try:
            self.sample()
            self.thread.start()
        except BaseException as error:
            self.failure = type(error).__name__
            self.closed = True
            os.close(self.fd); os.close(self.volume_fd)
            raise

    def close(self):
        if self.closed: raise ValueError('resource sampler already closed')
        self.stop.set()
        if self.thread.ident is not None: self.thread.join(5)
        if self.thread.is_alive(): raise RuntimeError('resource sampler exit unobserved')
        try:
            if self.failure is None:
                try: self.sample()
                except Exception as error: self.failure = type(error).__name__
            os.fsync(self.fd)
            actual, name = os.fstat(self.fd), self.path.lstat()
            if ((actual.st_dev, actual.st_ino) != self.raw_identity
                    or (name.st_dev, name.st_ino) != self.raw_identity
                    or actual.st_size != self.bytes or actual.st_nlink != 1
                    or actual.st_uid != os.geteuid() or actual.st_mode & 0o077):
                raise ValueError('original raw resource evidence replaced or changed')
            os.lseek(self.fd, 0, os.SEEK_SET)
            with os.fdopen(os.dup(self.fd), 'rb') as stream:
                self.raw_digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        except BaseException as error:
            self.failure = type(error).__name__
            raise
        finally:
            os.close(self.fd); os.close(self.volume_fd); self.closed = True
        return self.summary()

    def summary(self):
        if not self.closed:
            raise ValueError('sampler must exit before resource summary')
        epochs = copy.deepcopy(self.epochs)
        for epoch in epochs:
            first, last = epoch['first'], epoch['last']
            epoch['observed_cpu_ticks_delta'] = None if first is None else last['cpu_ticks']-first['cpu_ticks']
            epoch['observed_cpu_seconds_delta'] = (None if first is None else
                epoch['observed_cpu_ticks_delta']/self.ticks)
        return dict(schema=1, candidate=self.candidate, run_id=self.run_id, sampler_exited=True, failure=self.failure,
            coverage='partial' if self.failure or self.unknown or not self.vms else 'sampled_windows',
            interval_ms=self.interval_ms, samples=self.samples, unknown_observations=self.unknown,
            raw=dict(file=self.path.name, bytes=self.bytes, sha256=self.raw_digest),
            cpu_clock_ticks_per_second=self.ticks, epochs=epochs,
            service_group=dict(first=getattr(self, 'group_first', None), last=getattr(self, 'group_last', None),
                identity=self.group['identity'], scope='whole original service leaf, including inherited helpers; not sum of process peaks'),
            volume=dict(self.volume, identity=list(self.volume_identity),
                scope='whole original filesystem, all writers; excludes backing image allocation; sampled peak, not quota'),
            observer_overhead=dict(thread_cpu_ns=self.cpu_ns, sample_wall_ns=self.wall_ns,
                max_sample_wall_ns=self.max_wall_ns, max_observed_gap_ns=self.max_gap_ns,
                scope='sample calls only; excludes final fsync/hash and existing service storage sampler'),
            scope='only recorded process windows; startup/exit tails and between-sample peaks unobserved; '
                'kernel peaks cover process/cgroup lifetime; no internal phase timings, cold-cache or full E16 claim')
