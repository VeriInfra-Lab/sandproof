"""Bind approved capture limits to one fresh acceptance cohort.

This prepares test input only. It neither enables quotas nor certifies their
kernel readback, and never adopts or removes an existing snapshot layout.
"""
import copy
from pathlib import Path
import secrets

FIELDS = frozenset({'state_copy_bytes', 'memory_copy_bytes', 'disk_copy_bytes',
                    'api_timeout_ms', 'worker_timeout_ms', 'material_project'})
PROJECT_FIELDS = frozenset({'project_id', 'hard_bytes', 'hard_inodes', 'chattr', 'lsattr'})


def validate_profile(profile, count):
    if (type(profile) is not dict or set(profile) != {'schema_version', 'hosts'}
            or type(profile['schema_version']) is not int or profile['schema_version'] != 1
            or type(profile['hosts']) is not list or len(profile['hosts']) != count
            or count not in (1, 2)):
        raise ValueError('explicit approved capture profile for the whole cohort required')
    projects = set()
    for host in profile['hosts']:
        if type(host) is not dict or set(host) != FIELDS:
            raise ValueError('capture profile must contain limits, not old paths or layout identity')
        for key in ('state_copy_bytes', 'memory_copy_bytes', 'disk_copy_bytes'):
            limit = 10_000_000 if key == 'state_copy_bytes' else 2**63-1
            if type(host[key]) is not int or not 0 < host[key] <= limit:
                raise ValueError('unsupported capture copy limit')
        if (type(host['api_timeout_ms']) is not int or type(host['worker_timeout_ms']) is not int
                or not 1000 <= host['api_timeout_ms'] <= host['worker_timeout_ms'] <= 300_000):
            raise ValueError('finite original capture deadlines required')
        project = host['material_project']
        if type(project) is not dict or set(project) != PROJECT_FIELDS:
            raise ValueError('complete approved material project required')
        for key in ('project_id', 'hard_bytes', 'hard_inodes'):
            limit = 2**32-1 if key == 'project_id' else 2**63-1
            if type(project[key]) is not int or not 0 < project[key] <= limit:
                raise ValueError('positive representable material project bounds required')
        if project['project_id'] in projects:
            raise ValueError('host pools must not share a material project')
        projects.add(project['project_id'])
        for key in ('chattr', 'lsattr'):
            tool = project[key]
            if (type(tool) is not dict or set(tool) != {'path', 'sha256'}
                    or type(tool['path']) is not str or not Path(tool['path']).is_absolute()
                    or not Path(tool['path']).name
                    or str(Path(tool['path'])) != tool['path'] or '..' in Path(tool['path']).parts
                    or type(tool['sha256']) is not list or len(tool['sha256']) != 32
                    or any(type(value) is not int or not 0 <= value <= 255 for value in tool['sha256'])):
                raise ValueError('exact protected project tools required')
    return copy.deepcopy(profile['hosts'])


def validate_settings(settings, count):
    if type(settings) is not list or len(settings)!=count:
        raise ValueError('capture settings must bind the whole original host cohort')
    declarations=[]
    for value in settings:
        if type(value) is not dict or set(value)!=FIELDS|{'layout_parent','layout_binding'}:
            raise ValueError('complete fresh capture settings required')
        binding=value['layout_binding']
        if (type(value['layout_parent']) is not str
                or type(binding) is not list or len(binding)!=32
                or any(type(byte) is not int or not 0<=byte<=255 for byte in binding)):
            raise ValueError('exact layout path and binding bytes required')
        declarations.append({key:entry for key,entry in value.items() if key in FIELDS})
    validate_profile(dict(schema_version=1,hosts=declarations),count)
    return copy.deepcopy(settings)


def bind_settings(profile, pool_roots):
    """Fresh layout identity; approved hard limits and project IDs stay exact."""
    hosts = validate_profile(profile, len(pool_roots))
    roots = [Path(root) for root in pool_roots]
    if (any(not root.is_absolute() or root == Path('/')
            or '..' in root.parts or root.resolve(strict=False) != root for root in roots)
            or any(a == b or a.is_relative_to(b) or b.is_relative_to(a)
                   for index, a in enumerate(roots) for b in roots[index+1:])):
        raise ValueError('distinct canonical private acceptance roots required')
    for host, root in zip(hosts, roots):
        host['layout_parent'] = str(root / 'p')
        host['layout_binding'] = list(secrets.token_bytes(32))
    return hosts


def require_capacity(capture, native, slots):
    """Check the payload floor for a second capture with live File backing.

    Old bundle + private restored RAM/state + new Full source + new bundle
    coexist. The operator must additionally reserve metadata, allocation and
    cleanup slack; this floor does not attest that provision or a kernel quota.
    """
    if type(slots) is not int or slots < 1:
        raise ValueError('original retained-session capacity required')
    if capture['memory_copy_bytes'] < native['memory_mib'] * 1024**2:
        raise ValueError('capture memory cap cannot undercut the actual VM')
    if capture['disk_copy_bytes'] < Path(native['rootfs']).stat().st_size:
        raise ValueError('capture disk cap cannot undercut the original private disk')
    state_memory = capture['state_copy_bytes'] + capture['memory_copy_bytes']
    minimum = slots * (4 * state_memory + 2 * capture['disk_copy_bytes'])
    if minimum >= 2**63-1 or capture['material_project']['hard_bytes'] <= minimum:
        raise ValueError('approved project must exceed capture/restore overlap payload floor')
    return minimum


def original_chroot(pool_root):
    """Read the original protected role config, never a public ID or suffix."""
    from tools.system_acceptance import json_file, private_path
    root=Path(pool_root)
    config=json_file(private_path(root/'control/host.json'))
    native=config['native']
    result=Path(native['chroot_base'])
    if (Path(native['database'])!=root/'db/controller.db'
            or Path(native['node_state_dir'])!=root/'state'
            or result!=Path(config['capture']['layout_parent'])/'v'
            or result.parent!=root/'p'):
        raise ValueError('resource observation must use its original native capture layout')
    return result
