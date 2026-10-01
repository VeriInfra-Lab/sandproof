"""Conservative serial KVM acceptance budgets, derived before any dispatch."""
import math

MIB = 1024**2
GIB = 1024 * MIB
INPUT_BYTES = 4 * GIB
EXPANDED_BYTES = 8 * GIB
LOG_BYTES = 2 * MIB
CLEANUP_BYTES = GIB


def plan(entries, repetitions, builder_bytes, kernel_bytes, *, artifact_bytes=128 * MIB,
         input_bytes=INPUT_BYTES, expanded_bytes=EXPANDED_BYTES):
    if (not entries or type(repetitions) is not int or repetitions not in (1, 2)
            or any(type(v) is not int or v <= 0 for v in (builder_bytes, kernel_bytes, artifact_bytes))):
        raise ValueError('invalid acceptance inputs')
    for name, value, minimum, maximum in (
            ('input_bytes', input_bytes, 512, 8 * GIB),
            ('expanded_bytes', expanded_bytes, 10240, 32 * GIB)):
        if type(value) is not int or not minimum <= value <= maximum:
            raise ValueError('build budget outside service protocol: ' + name)
    if len({e['name'] for e in entries}) != len(entries):
        raise ValueError('duplicate task selection')
    req = [e['requirements'] for e in entries]
    for r in req:
        for key, lower, upper in (('cpus', 1, 255), ('memory_mib', 128, 16384), ('disk_mib', 512, 32768)):
            if type(r[key]) is not int or not lower <= r[key] <= upper:
                raise ValueError('task shape outside build protocol')
        timeout = r['build_timeout_sec']
        if (type(timeout) not in (int, float) or not math.isfinite(timeout)
                or not 1 <= timeout <= 3600):
            raise ValueError('task deadline outside build protocol')
    jobs = len(entries) * repetitions
    if jobs > 10000:
        raise ValueError('task count exceeds build history bound')
    disk = max(r['disk_mib'] for r in req) * MIB
    registry = sum(r['disk_mib'] * MIB + kernel_bytes for r in req) * repetitions
    history = jobs * (LOG_BYTES + 256 * 1024)
    workspace = builder_bytes + input_bytes + 2 * disk + kernel_bytes + LOG_BYTES + 64 * MIB + history
    # Original Dockerfile contexts are retained per trial, unlike the build
    # service's temporary input. Reserve payload, PAX headers and tar padding.
    contexts = repetitions * sum(10240 + sum(f['bytes'] + len(f['path'].encode()) + 4096
        for f in entry.get('files', []) if f['role'] == 'environment') for entry in entries)
    # Bounded guest/command evidence plus small controller-owned reports,
    # checkpoints and their aggregate copies (outside ArtifactBudget).
    evidence = jobs * (artifact_bytes + 4 * MIB)
    return {'jobs': jobs, 'uid_stride': max(100, 2 * jobs + 4),
            'registry_bytes': registry, 'workspace_bytes': workspace,
            'artifact_bytes_per_trial': artifact_bytes, 'trial_evidence_bytes': evidence,
            'context_reserve_bytes': contexts,
            'required_free_bytes': registry + workspace + evidence + contexts + 2 * CLEANUP_BYTES,
            'max_cpu': max(r['cpus'] for r in req), 'max_memory_mib': max(r['memory_mib'] for r in req),
            'max_disk_mib': disk // MIB, 'max_build_seconds': int(max(r['build_timeout_sec'] for r in req)),
            'input_bytes': input_bytes, 'expanded_bytes': expanded_bytes, 'log_bytes': LOG_BYTES,
            'cleanup_bytes': CLEANUP_BYTES,
            'worker_required_free_bytes': disk + expanded_bytes + 2 * input_bytes + 64 * MIB}
