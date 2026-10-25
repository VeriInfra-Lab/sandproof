"""The original acceptance declaration arithmetic, without physical admission.

RuntimeServices consumes these same functions before its actual statvfs/lease
checks. Planning output alone never certifies available space or a hard quota.
"""
import copy
from pathlib import Path
from tools.template_physical_storage import envelope

MIB=1024**2
MAX_ACTIVATION_BYTES=200*1024**3

def static_cache(pool, catalog, root):
    sizes = [item['kernel']['size_bytes'] + item['rootfs']['size_bytes'] for item in catalog['templates']]
    if not sizes:
        raise ValueError('explicit immutable registered catalog required')
    entries = len(sizes)
    return dict(namespace=pool['namespace'],root=str(root),max_entries=entries,max_downloads=1,
        max_entry_bytes=max(sizes),artifact_bytes=sum(sizes),temporary_bytes=max(sizes),
        metadata_bytes=4096+entries*2*(65536+4096),inode_budget=4+entries*5+5)


def profile_declaration(native_pools, catalogs, layout, *, registry, service_peaks, activation_ceiling_bytes=None):
    """Derive the exact declaration; this performs no physical admission."""
    profiles = [native['bounded_host']['physical_storage'] for native in native_pools]
    if any(profile != profiles[0] for profile in profiles[1:]):
        raise ValueError('one original shared physical declaration required')
    profile = copy.deepcopy(profiles[0])
    caches = profile['host_template_caches']
    if not caches:
        if len(native_pools) != 1:
            raise ValueError('dynamic two-pool acceptance requires explicit approved cache capacities')
        caches.append(static_cache(native_pools[0],catalogs[0],layout['hosts'][0]/'template-cache'))
    if len(caches) != len(native_pools):
        raise ValueError('one distinct physical cache for every exact host pool required')
    for index,native in enumerate(native_pools):
        matched=[cache for cache in caches if cache['namespace']==native['namespace']]
        if len(matched)!=1:
            raise ValueError('physical cache does not name its original pool')
        required=[item['kernel']['size_bytes']+item['rootfs']['size_bytes']
                  for item in catalogs[index]['templates']]
        if (len(required)>matched[0]['max_entries'] or max(required)>matched[0]['max_entry_bytes']
                or sum(required)>matched[0]['artifact_bytes']):
            raise ValueError('approved cache cannot contain static catalog; no capacity is raised')
        matched[0]['root']=str(layout['hosts'][index]/'template-cache')
    public=profile['public_build']
    registry_peak=registry['max_artifact_bytes_global']+registry['max_templates_global']*(65536+87)
    if public is not None:
        # Only the fresh layout/role owner changes. Original workspace, registry,
        # jobs, templates and cleanup provisions remain the supplied ceilings.
        if public['registry_bytes'] < registry_peak:
            raise ValueError('original registry budget cannot cover declared registry')
        public.update(registry_root=str(layout['registry']),registry_owner_uid=layout['center_identity'][0])
    else:
        service_peaks.append(dict(root=str(layout['registry']),owner_uid=layout['center_identity'][0],
            peak_bytes=registry_peak,inode_budget=8+5*registry['max_templates_global']))
    if profile['service_data']:
        raise ValueError('fresh service declarations required; existing service roots are not relocated')
    profile['service_data']=copy.deepcopy(service_peaks)
    peak,inodes=envelope(profile)
    if activation_ceiling_bytes is not None:
        if (type(activation_ceiling_bytes) is not int
                or not 0<activation_ceiling_bytes<=MAX_ACTIVATION_BYTES):
            raise ValueError('explicit prepared CI activation ceiling at most 200 GiB required')
        if 2*peak>activation_ceiling_bytes:
            raise ValueError('complete dual-service activation exceeds prepared CI ceiling; no quota is reduced')
    return profile


def service_declarations(layout, ids, paths, asset_bytes, redis_receipt, private_builds, captures):
    service_peaks=[dict(root=str(directory),owner_uid=uid,peak_bytes=budget,inode_budget=inodes)
        for directory,uid,budget,inodes in (
            (layout['center_control'],ids['center'][0],paths['center'].stat().st_size+8*MIB,32),
            (layout['assets'],ids['center'][0],asset_bytes+MIB,32),
            (layout['writer'],ids['center'][0],MIB,16),
            (layout['redis'],ids['redis'][0],redis_receipt['binary_bytes']+512*MIB,64),
            (layout['logs'],0,64*MIB,64),
            *((control,0,paths['host'].stat().st_size+8*MIB,32)
              for control in layout['host_controls']))]
    if private_builds is not None:
        service_peaks.append(dict(root=str(layout['build_control']),owner_uid=0,peak_bytes=8*MIB,inode_budget=32))
    for capture in captures:
        material=capture['material_project']
        service_peaks.append(dict(root=str(Path(capture['layout_parent'])/'s'),owner_uid=0,
            peak_bytes=material['hard_bytes'],inode_budget=material['hard_inodes']))
    return service_peaks
