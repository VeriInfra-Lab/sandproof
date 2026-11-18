"""Render one original lifecycle run after checking its raw resource evidence.

This checks engineering evidence consistency, not the truth of physical facts or
complete E16 acceptance. No inputs are merged or changed, and no product is run.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import sys

from tools.check_kvm_host import unique_keys
from tools.system_acceptance import json_file, private_path
from tools.template_build_guest import equal

MAX_RAW = 64 * 1024**2
COMPLETION_CONDITIONS = {
    'create': ('running',),
    'unload': ('unloaded', 'original_continued'),
    'resume': ('running', 'state_lost'),
    'destroy': ('cleaned',),
}
REQUIRED_SERVICE_ROLES = {'center', 'redis', 'host-0'}
SERVICE_ROLES = REQUIRED_SERVICE_ROLES | {'host-1'}
EPOCH_SAMPLE_STATES = {'observed', 'not_live', 'unknown', 'exited_during_sample'}
SERVICE_STATES_WITHOUT_EPOCH = EPOCH_SAMPLE_STATES - {'observed'} | {'not_started', 'not_ready'}


def uint(value):
    return type(value) is int and 0 <= value < 2**64


def duration(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def sample_cohorts(row, epochs):
    roles, vm_epochs, seen = set(), set(), set()
    for kind, key, limit in (('service', 'services', len(SERVICE_ROLES)),
                             ('vm', 'vms', len(epochs))):
        samples = row.get(key)
        if type(samples) is not list or len(samples) > limit:
            raise ValueError('bounded original service and VM sample lists required')
        for sample in samples:
            if type(sample) is not dict:
                raise ValueError('original process sample required')
            index = sample.get('epoch')
            state = sample.get('state')
            allowed_states = SERVICE_STATES_WITHOUT_EPOCH if index is None else EPOCH_SAMPLE_STATES
            if type(state) is not str or state not in allowed_states:
                raise ValueError('original resource observation state required')
            if index is not None:
                if (not uint(index) or index >= len(epochs) or index in seen
                        or epochs[index]['kind'] != kind):
                    raise ValueError('sample epoch must match its original service or VM list')
                seen.add(index)
                role = epochs[index]['role']
                if sample.get('role', role) != role:
                    raise ValueError('sample role differs from its original epoch')
            else:
                role = sample.get('role')
            if kind == 'vm':
                if index is None:
                    raise ValueError('every original VM sample requires its epoch')
                vm_epochs.add(index)
                continue
            if type(role) is not str or role not in SERVICE_ROLES or role in roles:
                raise ValueError('one sample for each original service role required')
            roles.add(role)
    if not REQUIRED_SERVICE_ROLES <= roles:
        raise ValueError('one sample for each original service role required')
    return roles, vm_epochs, seen


def checked_resources(report, samples):
    measurement = report['resource_measurements']
    raw = measurement['raw']
    candidate=report['measurement_identity']['candidate']['candidate']
    if (report.get('complete') is not True or not equal(report.get('schema'),1) or not equal(measurement.get('schema'),1)
            or measurement.get('sampler_exited') is not True or measurement.get('failure') is not None
            or type(measurement.get('samples')) is not int or not 1 <= measurement['samples'] <= 8192
            or type(raw.get('bytes')) is not int or not 0 < raw['bytes'] <= MAX_RAW
            or re.fullmatch('[0-9a-f]{64}',str(raw.get('sha256'))) is None
            or measurement['candidate'] != candidate
            or re.fullmatch('[0-9a-f]{32}',measurement['run_id']) is None
            or measurement.get('coverage') not in ('partial','sampled_windows')):
        raise ValueError('one completed run and sealed original resource stream required')
    epochs = measurement['epochs']
    if type(epochs) is not list or not 1 <= len(epochs) <= 64:
        raise ValueError('finite original process epochs required')
    observed = [dict(first=None,last=None,count=0,peak=None,kernel_peak=None,unknown=0,inactive=0) for _ in epochs]
    vm_index = 0
    for index, epoch in enumerate(epochs):
        if (not uint(epoch['index']) or epoch['index'] != index or not uint(epoch['identity']['pid'])
                or not uint(epoch['identity']['start_time_ticks'])
                or epoch['kind'] not in ('service','vm')
                or (epoch['role'].startswith('vm-')) != (epoch['kind']=='vm')
                or any(not uint(epoch[k]) for k in ('samples','unknown_samples','inactive_samples'))
                or re.fullmatch('center|redis|host-[01]|vm-[0-9]+',epoch['role']) is None):
            raise ValueError('exact original process epoch declaration required')
        if epoch['kind'] == 'vm':
            if epoch['role'] != f'vm-{vm_index}':
                raise ValueError('exact original VM epoch role required')
            vm_index += 1
    digest = hashlib.sha256(); size = count = unknown = 0; previous = None
    group_first = group_last = None
    service_roles = None
    prior_vms, seen_epochs = set(), set()
    volume = dict(samples=0,unknown_samples=0,baseline_used_bytes=None,sampled_peak_used_bytes=None,final_used_bytes=None)
    with private_path(samples).open('rb') as source:
        for line in iter(lambda:source.readline(16385),b''):
            if len(line) > 16384 or count >= 8192:raise ValueError('bounded raw resource rows required')
            size += len(line)
            if size > MAX_RAW:raise ValueError('bounded raw resource bytes required')
            digest.update(line)
            row = json.loads(line,object_pairs_hook=unique_keys)
            now = row['at_monotonic_ns']
            if (row['candidate'] != candidate or row['run_id'] != measurement['run_id']
                    or not uint(row['sequence']) or row['sequence'] != count or not uint(now)
                    or (previous is not None and now <= previous)):
                raise ValueError('original sequential monotonic resource stream required')
            previous = now
            roles, vms, seen = sample_cohorts(row, epochs)
            if service_roles is not None and roles != service_roles:
                raise ValueError('original service role cohort changed across resource rows')
            if not prior_vms <= vms:
                raise ValueError('original recorded VM epoch is missing from a later sample')
            service_roles, prior_vms = roles, vms
            seen_epochs.update(seen)
            for value in row['services']+row['vms']:
                index=value.get('epoch')
                state_name=value['state']
                if state_name in ('unknown','not_started','not_ready','exited_during_sample'):
                    unknown+=1
                    if index is not None:observed[index]['unknown']+=1
                if state_name=='not_live' and index is not None:observed[index]['inactive']+=1
                if state_name != 'observed':continue
                if index is None or any(not uint(value[k]) for k in
                        ('cpu_ticks','rss_bytes','kernel_process_peak_rss_bytes','at_monotonic_ns')) \
                        or not equal(value['identity'],epochs[index]['identity']) or value['at_monotonic_ns'] != now:
                    raise ValueError('bounded observed CPU/RSS values required')
                facts={k:value[k] for k in ('cpu_ticks','rss_bytes','kernel_process_peak_rss_bytes','at_monotonic_ns')}
                state=observed[index]
                if state['last'] is not None and facts['cpu_ticks'] < state['last']['cpu_ticks']:
                    raise ValueError('original CPU counter regressed')
                if state['first'] is None:state['first']=facts
                state['last']=facts;state['count']+=1
                state['peak']=facts['rss_bytes'] if state['peak'] is None else max(state['peak'],facts['rss_bytes'])
                state['kernel_peak']=(facts['kernel_process_peak_rss_bytes'] if state['kernel_peak'] is None
                    else max(state['kernel_peak'],facts['kernel_process_peak_rss_bytes']))
            group=row['service_group']
            if (not equal(row['service_group_identity'],measurement['service_group']['identity'])
                    or any(not uint(group[k]) for k in ('cpu_usage_usec','memory_current_bytes','kernel_group_peak_memory_bytes'))
                    or not equal(row['volume_identity'],measurement['volume']['identity'])):
                raise ValueError('original group/volume identity and counters required')
            if group_first is None:group_first=group
            group_last=group
            used=row['volume_used_bytes'];volume['final_used_bytes']=used
            if used is None:volume['unknown_samples']+=1;unknown+=1
            else:
                if not uint(used):raise ValueError('bounded original volume counter required')
                volume['samples']+=1
                if volume['baseline_used_bytes'] is None:volume['baseline_used_bytes']=used
                peak=volume['sampled_peak_used_bytes']
                volume['sampled_peak_used_bytes']=used if peak is None else max(peak,used)
            count+=1
    if size != raw['bytes'] or digest.hexdigest() != raw['sha256'] or count != measurement['samples']:
        raise ValueError('raw resource stream differs from original sealed summary')
    if seen_epochs != set(range(len(epochs))):
        raise ValueError('original epoch declaration is missing from the raw resource stream')
    if (not equal(group_first,measurement['service_group']['first']) or not equal(group_last,measurement['service_group']['last'])
            or any(not equal(value,measurement['volume'][key]) for key,value in volume.items())
            or not equal(unknown,measurement['unknown_observations'])):
        raise ValueError('group, volume or missing-data summary differs from original samples')
    expected_coverage='partial' if unknown or not any(e['kind']=='vm' for e in epochs) else 'sampled_windows'
    if measurement['coverage'] != expected_coverage:
        raise ValueError('missing observations cannot become complete resource coverage')
    ticks=measurement['cpu_clock_ticks_per_second']
    if not uint(ticks) or ticks==0:raise ValueError('original CPU clock tick rate required')
    for state,epoch in zip(observed,epochs):
        delta=None if state['first'] is None else state['last']['cpu_ticks']-state['first']['cpu_ticks']
        seconds=epoch['observed_cpu_seconds_delta']
        if (state['count'] != epoch['samples'] or not equal(state['first'],epoch['first']) or not equal(state['last'],epoch['last'])
                or state['unknown'] != epoch['unknown_samples'] or state['inactive'] != epoch['inactive_samples']
                or not equal(state['peak'],epoch['sampled_peak_rss_bytes'])
                or not equal(state['kernel_peak'],epoch['kernel_process_peak_rss_bytes'])
                or not equal(delta,epoch['observed_cpu_ticks_delta'])
                or (seconds is not None and type(seconds) not in (int,float))
                or (None if delta is None else delta/ticks) != seconds):
            raise ValueError('process summary differs from its original resource samples')
    return measurement


def render(report, samples):
    measurements=checked_resources(report,samples)
    candidate=report['measurement_identity']['candidate']['candidate']
    scenario=report['scenario']
    if (re.fullmatch('[0-9a-f]{40}',candidate) is None
            or re.fullmatch('[a-z0-9-]{1,128}',scenario) is None):
        raise ValueError('exact candidate and original scenario required')
    rows=report['operation_timings']
    if type(rows) is not list or not 1 <= len(rows) <= 128:raise ValueError('bounded original timings required')
    timing = {(operation, condition): [] for operation, conditions in COMPLETION_CONDITIONS.items()
              for condition in conditions}
    incomplete = 0
    for row in rows:
        if type(row) is not dict or type(row.get('complete')) is not bool:
            raise ValueError('original operation timing required')
        key = (row.get('operation'), row.get('completion_condition'))
        if any(type(value) is not str for value in key) or key not in timing:
            raise ValueError('original operation and completion condition required')
        before = row.get('send_to_accept_seconds')
        after = row.get('accept_to_public_completion_seconds')
        total = row.get('send_to_public_completion_seconds')
        if not row['complete']:
            if (before is not None and not duration(before)) or after is not None or total is not None:
                raise ValueError('incomplete original operation cannot have a completion duration')
            incomplete += 1
            continue
        if not all(duration(value) for value in (before, after, total)):
            raise ValueError('finite original acceptance and completion durations required')
        # The source calculates three float subtractions on monotonic readings.
        if not math.isclose(before + after, total, rel_tol=0, abs_tol=2 * math.ulp(total)):
            raise ValueError('original acceptance and completion durations do not add up')
        timing[key].append(total)
    out=['# 生命周期工程测量','',f'候选：`{candidate}`；场景：`{scenario}`。',
        '',f"资源流：{measurements['samples']} 个样本；覆盖：`{measurements['coverage']}`；"
        f"未知观察：{measurements['unknown_observations']}。",'',
        '| 操作 | 公开终局 | 完成样本数 | p50 秒 | p95 秒 |','|---|---|---:|---:|---:|']
    for (name, condition), values in timing.items():
        values.sort()
        p50,p95=((f'{values[math.ceil(q*len(values))-1]:.6f}' for q in (.5,.95)) if values else ('未知','未知'))
        out.append(f'| {name} | {condition} | {len(values)} | {p50} | {p95} |')
    out += ['',f'最近秩统计；未完成操作 {incomplete} 项不进入分位数。样本数不支持总体延迟保证。','',
        '| 进程角色/序段 | PID/start | 已观察 CPU 秒差 | 采样 RSS 峰值 MiB | 内核报告 HWM MiB | 有效/未知样本 |',
        '|---|---|---:|---:|---:|---|']
    def number(value,scale=1):return '未知' if value is None else f'{value/scale:.6f}'
    for epoch in measurements['epochs']:
        identity=epoch['identity']
        out.append(f"| {epoch['role']}/{epoch['index']} | {identity['pid']}/{identity['start_time_ticks']} | "
            f"{number(epoch['observed_cpu_seconds_delta'])} | {number(epoch['sampled_peak_rss_bytes'],1024**2)} | "
            f"{number(epoch['kernel_process_peak_rss_bytes'],1024**2)} | {epoch['samples']}/{epoch['unknown_samples']} |")
    volume=measurements['volume']
    out += ['',f"原文件系统已用字节：基线 {volume['baseline_used_bytes']}，采样峰值 {volume['sampled_peak_used_bytes']}，"
        f"末次 {volume['final_used_bytes']}；未知样本 {volume['unknown_samples']}。",'',
        'CPU 差仅覆盖同一进程已观察窗口；进程重启分别记账。RSS 采样峰值不保证捕获间隔中的峰值，'
        'HWM 是内核报告的进程全生命周期值。cgroup 内存计费、服务目录观察和采集开销见原 JSON；不累加各进程独立峰值。',
        '', '整卷包含其它写者，不能归因于某个沙箱，也不是项目配额或 CoW 共享块精确归属。',
        '', '未控制冷/热缓存，未取得内部 freeze/capture/copy/sync 等分段；本报告不是完整 E16 或系统验收证明。','']
    return '\n'.join(out)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report',type=Path,required=True)
    parser.add_argument('--samples',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    try:
        result=render(json_file(private_path(args.report),limit=16*1024**2),args.samples)
        fd=os.open(args.output,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
        with os.fdopen(fd,'w') as output:output.write(result);output.flush();os.fsync(output.fileno())
    except Exception as error:
        print('lifecycle_measurement_report: '+type(error).__name__,file=sys.stderr)
        return 1
    return 0


if __name__=='__main__':raise SystemExit(main())
