"""E13 first Resume receipt loss; decoded correlation and read-only original facts.

No Host audit/restart occurs inside the center-only fault window. This is a
finite consumer, not an authorization provider, VM runner or durability proof.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

from contextlib import closing
import copy
import json
from pathlib import Path
import sqlite3

from _internal_fault_relay import _fields, _u64, accepted_request, resumed_observation, unloaded_observation, same_json
from lifecycle_unload_receipt_loss import UnloadRestartObserver, UnloadReceiptRestart, _intent, _processes
from lifecycle_unload_runtime import checked_source_records, original_json, original_process, remaining
from tools.check_kvm_host import unique_keys
from tools.runtime_create_recovery import CURRENT_VERSIONS, original_create_identity
from tools.system_acceptance import private_path


def target_row(audit,session):
    _fields(audit,'schema_version versions deployment revision sandbox_highwater operation_highwater reserved_resources nodes retention_lanes sandboxes')
    if (type(audit['schema_version']) is not int or audit['schema_version']!=3
            or not same_json(audit['versions'],CURRENT_VERSIONS) or type(audit['sandboxes']) is not list):
        raise ValueError('current original native center audit required')
    rows=[r for r in audit['sandboxes'] if r['create']['binding']['session']==session]
    if len(rows)!=1:raise ValueError('one original Resume session required')
    row=rows[0]
    _fields(row,'create private operations terminating released phase last_running terminal_confirmation')
    if row['private'] is not False or row['terminating'] is not False or row['released'] is not False or row['terminal_confirmation'] is not None:
        raise ValueError('original nonterminal retained session required')
    return row


class ResumeRestartObserver(UnloadRestartObserver):
    def __init__(self,*,node,acceptance,create_identity,lifetime,baseline):
        super().__init__(node=node,acceptance=acceptance,create_identity=create_identity,lifetime=lifetime)
        row=target_row(baseline,acceptance['session'])
        if row['phase']!='unloaded' or len(row['operations'])!=2:
            raise ValueError('completed original first Unload required before Resume')
        source=row['operations'][-1]['unloaded']
        if source is None or source['outcome']['state']!='unloaded':raise ValueError('original complete source required')
        if not unloaded_observation(_intent(row['operations'][-1]),source,node):
            raise ValueError('complete native original Unload binding required')
        self.baseline=copy.deepcopy(baseline);self.source=copy.deepcopy(source['outcome']['facts'])
        if not same_json({k:self.source[k] for k in lifetime},lifetime):raise ValueError('source lifetime changed')
        if not same_json(original_create_identity(row),create_identity):raise ValueError('original Create changed')

    def _facts(self,value):
        _fields(value,'started_at_ms end_at_ms max_end_at_ms startup_deadline_ms observed_at_ms')
        for v in value.values():_u64(v)
        if (not same_json({k:value[k] for k in self.lifetime},self.lifetime)
                or not value['started_at_ms']<=value['observed_at_ms']<value['end_at_ms']):
            raise ValueError('Resume changed original source lifetime')

    def hold(self,observation,processes):
        self._phase('initial');request=observation['request']
        if not resumed_observation(request,observation,self.node):raise ValueError('actual current observed Resume required')
        binding=request['request']['binding'];op=request['request']['operation']
        if (not same_json(dict(session=binding['session'],key=binding['key'],accepted_version=request['accepted_version']),self.acceptance)
                or request['accepted_version']!=3
                or any(not same_json(op[k],self.source[k]) for k in ('source_incarnation','capture','manifest_digest'))):
            raise ValueError('first Resume changed original acceptance or source')
        self._facts(observation['readiness']['facts'])
        self.before=_processes(processes,'center redis host-0');self.observation=copy.deepcopy(observation);self.phase='held'

    def _record(self,audit):
        row=target_row(audit,self.acceptance['session']);prior=target_row(self.baseline,self.acceptance['session'])
        if audit['deployment']!=self.node['deployment_id'] or len(row['operations'])!=3:
            raise ValueError('one original first Resume obligation required')
        if (not same_json(row['create'],prior['create']) or not same_json(row['operations'][:2],prior['operations'])
                or not same_json(_intent(row['operations'][-1]),self.observation['request'])):
            raise ValueError('Create/source history or complete Resume intent changed')
        accepted_request(_intent(row['operations'][-1]),self.node)
        for name in ('terminal','unloaded','create_refusal'):
            if row['operations'][-1][name] is not None:raise ValueError('Resume has an unrelated result')
        keys=[op for r in audit['sandboxes'] for op in r['operations']
              if same_json(op['binding']['key'],self.acceptance['key'])]
        if len(keys)!=1:raise ValueError('original full Resume key is not unique')
        peers=[r for r in audit['sandboxes'] if r['create']['binding']['session']!=self.acceptance['session']]
        expected=[r for r in self.baseline['sandboxes'] if r['create']['binding']['session']!=self.acceptance['session']]
        if not same_json(peers,expected):raise ValueError('another session changed inside center-only window')
        if not same_json(audit['reserved_resources'],self.baseline['reserved_resources']):
            raise ValueError('original running/material obligations refunded or changed')
        return row

    def center_stopped(self,audit,survivors):
        self._phase('held');survivors=_processes(survivors,'redis host-0')
        if not same_json(survivors,{k:self.before[k] for k in survivors}):raise ValueError('Host or Redis changed')
        row=self._record(audit);prior=target_row(self.baseline,self.acceptance['session'])
        if (row['phase']!='resuming' or row['operations'][-1]['running'] is not None
                or not same_json(row['last_running'],prior['last_running'])):
            raise ValueError('center must retain Resume without adopting its running reply')
        self.pending=copy.deepcopy(row)
        names='schema_version versions deployment sandbox_highwater operation_highwater reserved_resources'.split()
        self.center_facts=copy.deepcopy({k:audit[k] for k in names});self.node_facts=self._stable_nodes(audit);self.phase='stopped'

    def discarded(self,evidence):
        self._phase('stopped')
        _fields(evidence,'observation held dropped response_attempts observed_at_ms')
        samples=evidence['observed_at_ms']
        if type(samples) is not list or len(samples)!=evidence['held'] or not 0<len(samples)<=8:
            raise ValueError('all bounded actual Resume observation times required')
        for sample in samples:
            facts=copy.deepcopy(self.observation['readiness']['facts']);facts['observed_at_ms']=sample;self._facts(facts)
        reduced={k:v for k,v in evidence.items() if k!='observed_at_ms'}
        super().discarded(reduced);self.loss=copy.deepcopy(evidence)

    def recovered(self,audit):
        self._phase('restarted');row=self._record(audit);facts=row['operations'][-1]['running'];self._facts(facts)
        expected=copy.deepcopy(self.pending);expected['phase']='running'
        expected['last_running']=copy.deepcopy(facts);expected['operations'][-1]['running']=copy.deepcopy(facts)
        if (not same_json(row,expected)
                or not same_json({k:audit[k] for k in self.center_facts},self.center_facts)
                or not same_json(self._stable_nodes(audit),self.node_facts)):
            raise ValueError('Resume recovery changed original history, identity or reservations')
        self.phase='recovered'
        return copy.deepcopy(dict(scope='e13-first-resume-center-receipt-loss',acceptance=self.acceptance,
            observation=self.observation,recovered_running=facts,loss=self.loss,before=self.before,after=self.after,
            observation_time='each real valid readonly time retained; not frozen across requests'))


def observe_candidate(runtime,intent,deadline):
    """Read original SQLite + protected claims/procfs; never stop/audit Host."""
    from server_bounded_churn import live_identity
    native=runtime.services.native_pools[0];config=runtime.services.host_configs[0]
    database=private_path(native['database']);stamp=(database.stat().st_dev,database.stat().st_ino)
    with closing(sqlite3.connect(database.as_uri()+'?mode=ro',uri=True,timeout=min(2,remaining(deadline)))) as conn:
        conn.execute('BEGIN')
        records=checked_source_records(conn,config,deadline)
        matched=[r for r in records if r['restore'] is not None and same_json(r['restore']['intent'],intent)]
        if len(matched)!=1:raise ValueError('one complete original Resume claim required')
        record=matched[0];claim=record['restore'];activation=claim['activation']
        if claim['material_use'] is not True or activation is None or record['capture']['completed'] is None:
            raise ValueError('actual activation marker and original backing pin required')
        action=activation['command']['logical']['command']
        if set(action)!={'ActivateVm'}:raise ValueError('original activation marker kind differs')
        fields=action['ActivateVm'];local=claim['local']
        for value in (fields['sandbox'],fields['generation'],fields['epoch'],fields['vm'],local['sandbox'],local['owner'],claim['generation']):
            _u64(value,positive=True)
        _u64(activation['identity']['attempt']);_u64(activation['command']['attempt'])
        if (type(activation['identity']['protocol_version']) is not int or activation['identity']['protocol_version']!=1
                or activation['identity']['operation']!='op-'+native['namespace']+'-'+str(local['sandbox'])+'-'+str(local['operation'])
                or activation['identity']['attempt']!=activation['command']['attempt']
                or fields['sandbox']!=local['sandbox'] or fields['generation']!=claim['generation']
                or activation['identity']['owner']!='tenant-'+str(local['owner'])
                or activation['identity']['sandbox']!='sb-'+native['namespace']+'-'+str(local['sandbox'])
                or activation['identity']['generation']!=claim['generation']
                or activation['identity']['epoch']!=fields['epoch']):
            raise ValueError('activation generation or local session differs')
        resource='vm-'+native['namespace']+'-'+str(fields['vm'])
        if activation['identity']['resource']!={'kind':'vm','id':resource}:
            raise ValueError('original activation resource differs')
        count,largest=conn.execute('SELECT count(*),max(length(context_json)) FROM controller_host_slots').fetchone()
        cap=config['native']['maintenance']['capacity']['max_sql_value_bytes']
        if not 0<count<=native['bounded_host']['maintenance']['max_host_slots'] or largest>cap:
            raise ValueError('bounded original physical contexts required')
        contexts=[json.loads(raw,object_pairs_hook=unique_keys) for _,raw in conn.execute('SELECT position,context_json FROM controller_host_slots')]
        contexts=[c for c in contexts if c['binding']['sandbox']==local['sandbox']
            and c['binding']['generation']==claim['generation'] and c['binding']['vm']==fields['vm']
            and c['binding']['owner']==local['owner']]
        if len(contexts)!=1:raise ValueError('exact original restored context required')
    if (database.stat().st_dev,database.stat().st_ino)!=stamp:raise ValueError('original SQLite replaced')
    source=original_json(Path(native['chroot_base'])/'.claims'/resource/'record.json',16384)
    if source['resource_id']!=resource or not original_process(source):raise ValueError('original restored VMM missing')
    def ticks(pid):
        return int((Path('/proc')/str(pid)/'stat').read_text().rsplit(')',1)[1].split()[19])
    physical=live_identity(native,contexts[0],ticks)
    if (any(not same_json(physical[k],source[k]) for k in ('resource_id','pid','start_time_ticks'))
            or not original_process(source)):
        raise ValueError('restored VMM changed during observation')
    return copy.deepcopy(dict(claim=claim,context=contexts[0],source=source,physical=physical,
        manifest=record['capture']['completed'],scope='one live original candidate and pin snapshot; not a complete API invocation trace'))


class ResumeReceiptRestart(UnloadReceiptRestart):
    barrier_name='resume_barrier'

    def arm(self,create_acceptance,saved,deadline):
        if not self.started or self.closed or 'preparation' in self.evidence:raise ValueError('one fresh Resume gate required')
        self._configs();before=self.services.processes.observe()
        audit=self.services.planned_audit('center',ready_deadline=deadline);self._ready(deadline)
        after=self.services.processes.observe()
        if any(not same_json(before[k],after[k]) for k in ('host-0','redis')):raise ValueError('preparatory audit changed survivors')
        row=target_row(audit,create_acceptance['session']);identity=original_create_identity(row)
        if (row['phase']!='unloaded' or len(row['operations'])!=2
                or not same_json(dict(session=identity['binding']['session'],key=identity['binding']['key'],accepted_version=identity['accepted_version']),create_acceptance)
                or saved['phase']!='unloaded' or saved['current_version']!=2
                or not same_json(saved['unload_outcome'],row['operations'][-1]['unloaded']['outcome'])
                or not same_json(saved['operation'],row['operations'][-1]['binding']['key'])):
            raise ValueError('public original source and native first Unload differ')
        self.baseline=copy.deepcopy(audit);self.create_identity=identity
        self.evidence['preparation']=dict(before=before,after=after,kind='planned-center-native-read-before-resume-fault')
        self._healthy();self._remaining(deadline);self._gate().arm();return self.evidence

    def _make_observer(self,acceptance,running):
        return ResumeRestartObserver(node=self.node,acceptance=acceptance,create_identity=self.create_identity,
            lifetime=running['lifetime'],baseline=self.baseline)

    def _loss_evidence(self):
        return self.relay.resume_loss_evidence()

    def _held(self,acceptance,deadline):
        self.candidate=observe_candidate(self.runtime,self.observer.observation['request'],deadline)
        self.evidence['candidate_at_hold']=copy.deepcopy(self.candidate)

    def _restarted(self,acceptance,deadline):
        current=observe_candidate(self.runtime,self.observer.observation['request'],deadline)
        if not same_json(current,self.candidate):raise ValueError('Resume recovery replaced original candidate, activation or pin')
        self.evidence['candidate_after_center_restart']=current

    def _validate_public(self,public):
        _fields(public,'session operation current_version phase unload_outcome create_refusal')
        expected=self.observer.acceptance
        if (public['phase']!='running' or public['unload_outcome'] is not None or public['create_refusal'] is not None
                or not same_json(dict(session=public['session'],key=public['operation'],accepted_version=public['current_version']),expected)):
            raise ValueError('recovered public Running changed original Resume')

    def assert_candidate(self,physical):
        if (not same_json(physical['source'],self.candidate['source'])
                or not same_json(physical['context'],self.candidate['context'])
                or physical['material_use_pin'] is not True):
            raise ValueError('post-window planned Host audit found another restored candidate')
        return dict(same_original_candidate=True,original_pin_retained=True,complete_execution_trace_claimed=False)
