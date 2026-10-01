"""Shared acceptance budgets, not production defaults or a host-wide cgroup cap.

Runtime transfer/deadline values come from the existing task profile. Plan for
both complete controller quotas, even when a task runner dispatches serially.
The service/relay allowance and host headroom must still be measured on KVM.
"""
import copy
import hashlib
import json
from pathlib import Path

from tools.generic_capacity import MIB
from tools.template_build_config import HOST_MEMORY_OVERHEAD_MIB, UPLOAD_ENVELOPE_BYTES, validate_capacity

ROOT = Path(__file__).resolve().parents[1]
PROFILE = ROOT / 'benchmarks/template-service-budgets.json'
RUNTIME_PROFILE = ROOT / 'benchmarks/local-service-budgets.json'


def load():
    return json.loads(PROFILE.read_text())


def positive(values, name):
    value = values[name]
    if type(value) is not int or value <= 0:
        raise ValueError('positive acceptance budget required: ' + name)
    return value


def budgets(capacity, builder_bytes, kernel_bytes, *, profile=None):
    profile = copy.deepcopy(load() if profile is None else profile)
    if type(profile.get('schema_version')) is not int or profile['schema_version'] != 1:
        raise ValueError('unsupported acceptance profile')
    expected = {'runtime': {'max_active', 'max_vcpus', 'max_host_memory_bytes', 'validation_timeout_secs'},
                'builder': {'cpu_count', 'memory_mib', 'max_pending', 'file_idle_seconds'},
                'host': {'control_cpu_allowance', 'cpu_headroom', 'control_memory_allowance_bytes', 'memory_headroom_bytes'}}
    if set(profile) != {'schema_version', *expected}:
        raise ValueError('unknown acceptance section')
    for section, fields in expected.items():
        if set(profile[section]) != fields:
            raise ValueError('invalid acceptance section: ' + section)
        for field in fields:
            positive(profile[section], field)
    if type(builder_bytes) is not int or builder_bytes <= 0 or type(kernel_bytes) is not int or kernel_bytes <= 0:
        raise ValueError('positive fixed artifact sizes required')
    r, b, h = (profile[key] for key in ('runtime', 'builder', 'host'))
    target = capacity['max_disk_mib'] * MIB
    runtime = json.loads(RUNTIME_PROFILE.read_text())
    # Include the trusted fallback base, without claiming that four of the
    # largest fallback images must fit simultaneously in the task pool.
    runtime_disk = max(r['max_active'] * target, builder_bytes)
    runtime.update(max_active_global=r['max_active'], max_active_per_tenant=r['max_active'],
                   max_disk_bytes_global=runtime_disk + capacity['cleanup_bytes'],
                   max_disk_bytes_per_tenant=runtime_disk, cleanup_reserve_bytes=capacity['cleanup_bytes'],
                   startup_timeout_secs=capacity['max_build_seconds'])
    runtime_compute = {'max_vcpus_global': r['max_vcpus'], 'max_vcpus_per_tenant': r['max_vcpus'],
                       'max_memory_bytes_global': r['max_host_memory_bytes'], 'max_memory_bytes_per_tenant': r['max_host_memory_bytes']}
    builder_memory = (b['memory_mib'] + HOST_MEMORY_OVERHEAD_MIB) * MIB
    # Supervisor runs one build at a time; admitted pending jobs nevertheless
    # retain their full disk reservations until exact cleanup is confirmed.
    builder = {'max_active_global': 1, 'max_active_per_tenant': 1,
               'max_disk_bytes_global': builder_bytes + capacity['cleanup_bytes'],
               'max_disk_bytes_per_tenant': builder_bytes, 'cleanup_reserve_bytes': capacity['cleanup_bytes'],
               'max_timeout_secs': capacity['max_build_seconds'] + 60,
               'max_lifetime_secs': capacity['max_build_seconds'] + 60,
               'startup_timeout_secs': capacity['max_build_seconds'], 'expiry_scan_seconds': 1,
               'data_transfer': dict(runtime['data_transfer'],
                    file_upload_bytes=capacity['input_bytes'] + UPLOAD_ENVELOPE_BYTES,
                    file_download_bytes=max(target, capacity['log_bytes'], 128 * 1024),
                    file_idle_seconds=b['file_idle_seconds'])}
    builder_compute = {'max_vcpus_global': b['cpu_count'], 'max_vcpus_per_tenant': b['cpu_count'],
                       'max_memory_bytes_global': builder_memory, 'max_memory_bytes_per_tenant': builder_memory}
    pending = min(b['max_pending'], capacity['jobs'])
    history = capacity['jobs'] * (capacity['log_bytes'] + 256 * 1024)
    workspace = pending * (capacity['workspace_bytes'] - history) + history
    limits = {'max_cpu': capacity['max_cpu'], 'max_memory_mib': capacity['max_memory_mib'],
              'max_disk_mib': capacity['max_disk_mib'], 'timeout_seconds': capacity['max_build_seconds'],
              'max_jobs': capacity['jobs'], 'max_jobs_per_tenant': capacity['jobs'],
              'max_pending': pending, 'max_pending_per_tenant': pending,
              'workspace_bytes': workspace, 'workspace_bytes_per_tenant': workspace,
              'max_input_bytes': capacity['input_bytes'], 'max_expanded_bytes': capacity['expanded_bytes'],
              'max_entries': 200000, 'max_log_bytes': capacity['log_bytes']}
    registry = {'max_templates_global': capacity['jobs'], 'max_templates_per_tenant': capacity['jobs'],
                'max_artifact_bytes_global': capacity['registry_bytes'],
                'max_artifact_bytes_per_tenant': capacity['registry_bytes'],
                'validation_timeout_secs': r['validation_timeout_secs']}
    base = {'resources': {'cpu_count': b['cpu_count'], 'host_memory_bytes': builder_memory,
                          'disk_bytes': builder_bytes}, 'rootfs': {'size_bytes': builder_bytes}}
    # Reuse the service's actual cross-pool contract rather than a parallel
    # model of what its publication and transport layers will accept.
    validate_capacity(runtime | {'templates': {'compute_limits': runtime_compute}, 'template_registry': registry},
                      builder | {'templates': {'compute_limits': builder_compute}}, limits, base,
                      kernel_bytes=kernel_bytes, largest_tool_bytes=capacity['input_bytes'])
    host = {'minimum_cpus': r['max_vcpus'] + b['cpu_count'] + h['control_cpu_allowance'] + h['cpu_headroom'],
            'minimum_available_memory_bytes': r['max_host_memory_bytes'] + builder_memory
                + h['control_memory_allowance_bytes'] + h['memory_headroom_bytes'],
            # The former serial estimate omitted extra live runtime disks.
            # Builder/candidate copies are already charged in workspace.
            'minimum_free_disk_bytes': capacity['required_free_bytes'] + runtime_disk
                + workspace - capacity['workspace_bytes'],
            'runtime_disk_bytes': runtime_disk, 'runtime_vmm_memory_bytes': r['max_host_memory_bytes'],
            'builder_vmm_memory_bytes': builder_memory, **h,
            'control_allowance_enforced_by_cgroup': False}
    return {'runtime': runtime, 'builder': builder, 'runtime_compute': runtime_compute,
            'builder_compute': builder_compute, 'builder_shape': {'cpu_count': b['cpu_count'],
            'memory_mib': b['memory_mib'], 'host_memory_bytes': builder_memory},
            'registry': registry, 'limits': limits, 'host': host, 'profile': profile,
            'profile_canonical_sha256': hashlib.sha256(json.dumps(profile, sort_keys=True).encode()).hexdigest(),
            'runtime_profile_sha256': hashlib.sha256(RUNTIME_PROFILE.read_bytes()).hexdigest()}


def check_headroom(deployment, *, cpus, available_memory, free_disk):
    host = deployment['host']
    for actual, field in ((cpus, 'minimum_cpus'), (available_memory, 'minimum_available_memory_bytes'),
                          (free_disk, 'minimum_free_disk_bytes')):
        if type(actual) is not int or actual < host[field]:
            raise ValueError('host headroom insufficient: ' + field)
