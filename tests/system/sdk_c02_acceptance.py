"""Run only the two existing C02 probes on an approved isolated Linux/KVM host.

No host execution is authorized by invoking this CLI. Supply an independently
approved exact commit, double-artifact receipt, and existing host profile.
Every run retains a private directory; --export-run reads a safe summary only
and never replays an interrupted profile or attempts host-direct cleanup.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]

import argparse
from contextlib import ExitStack, closing
import fcntl
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools import check_kvm_host as host
from tools.system_acceptance import (
    Failure, PHASES, PROBES, digest, json_file, new_report, read_report,
    record_failure, require, safe_export, valid_sandbox_id, validate_probe, write_once,
)
# Read-only --export-run remains safe under -O. Execution helpers must not
# import guarded resource modules until assertions are known to be enabled.
if not sys.flags.optimize:
    from server_process_crash import (
        complete_runtime, free_loopback_port, process_start_ticks, provision_runtime,
        retired_sandboxes,
    )
    from _process_utils import stop_child
    from _runtime_services import RuntimeServices
    from _center_host_services import terminal_result
    from tools import ci_e2e_artifacts as artifacts

# Existing system supervisors import this public helper.
sha256_file = digest

PHYSICAL_SLOTS = 2
EXPECTED_SANDBOXES = 4
CHECKOUT = Path(__file__).resolve().parents[2]
SOURCE_FILES = {
    "runner": "tests/system/sdk_c02_acceptance.py",
    "assertions_guard": "tests/system/_assertions_required.py",
    "report_helper": "tools/system_acceptance.py",
    "preflight_helper": "tools/check_kvm_host.py",
    "server_helper": "tests/system/server_process_crash.py",
    "complete_runtime_helper": "tests/system/_complete_runtime.py",
    "resource_evidence_helper": "tests/system/_resource_evidence.py",
    "physical_profile_helper": "tools/template_physical_storage.py",
    "runtime_services_helper": "tests/system/_runtime_services.py",
    "role_fixture_helper": "tests/system/_center_host_services.py",
    "artifact_helper":"tools/ci_e2e_artifacts.py",
    "recovery_helper":"tools/runtime_create_recovery.py",
    "service_cgroup_helper":"tools/service_cgroup.py",
    "template_helper":"tests/system/server_templates.py",
    "process_helper":"tests/system/_process_utils.py",
    "sdk_server_smoke.py": "tests/system/sdk_server_smoke.py",
    "sdk_data_contract.py": "tests/system/sdk_data_contract.py",
    "sdk_data_samples.py": "tests/compat/sdk_data_samples.py",
}


def processes_in_uid_range(first, last):
    _require_assertions(Failure)
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            status = (entry / "status").read_text()
        except (FileNotFoundError, ProcessLookupError):
            continue
        for line in status.splitlines():
            if line.startswith("Uid:"):
                if any(first <= int(uid) <= last for uid in line.split()[1:]):
                    found.append(int(entry.name))
                break
        else:
            raise Failure("resource_unknown")
    return sorted(found)


def durable_rows(database, namespace):
    """Observe current SQL or exact quiescent retirement; never infer IDs from logs."""
    _require_assertions(Failure)
    require(database.is_file() and not database.is_symlink(), "database_unknown")
    try:
        with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=2)) as connection:
            connection.execute("BEGIN")
            rows = connection.execute(
                "SELECT sandbox_id, owner_id, state_blob FROM sandboxes ORDER BY sandbox_id"
            ).fetchall()
            retired = retired_sandboxes(connection, namespace)
        observed = {}
        for ident, owner, raw in rows:
            require(valid_sandbox_id(ident, namespace) and owner == "tenant-1", "identity_unknown")
            value = json.loads(raw, object_pairs_hook=host.unique_keys)
            require(isinstance(value, dict) and type(value.get("owner")) is int
                    and value["owner"] == 1 and value.get("phase") in
                    ("Starting", "Running", "Pausing", "Paused", "Revoking", "Destroyed")
                    and type(value.get("terminal")) is bool, "database_unknown")
            for field in ("current_vm", "net_slot"):
                number = value[field]
                require(number is None or (type(number) is int and 0 <= number < 2**64), "database_unknown")
            observed[ident] = value
        for ident in retired:
            require(valid_sandbox_id(ident, namespace) and ident not in observed, "identity_unknown")
            # This is an observation tag, not a fabricated SandboxRecord or
            # worker receipt. The exact terminal reducer already retired it.
            observed[ident] = {"phase": "Retired", "terminal": True,
                               "current_vm": None, "net_slot": None, "owner": 1}
        require(len(observed) <= EXPECTED_SANDBOXES, "identity_unknown")
        return observed
    except (sqlite3.Error, ValueError, TypeError, KeyError) as error:
        raise Failure("database_unknown") from error


def destroyed(rows):
    return all(value["phase"] in ("Destroyed", "Retired") and value["terminal"]
               and value["current_vm"] is None and value["net_slot"] is None
               for value in rows.values())


def wait_for_destroyed(database, namespace, identities, *, timeout=30):
    _require_assertions(Failure)
    deadline = time.monotonic() + timeout
    while True:
        rows = durable_rows(database, namespace)
        require(set(rows) == set(identities), "identity_unknown")
        if destroyed(rows):
            return rows
        if time.monotonic() >= deadline:
            raise Failure("terminal_state_invalid")
        time.sleep(0.2)


def entries(path):
    try:
        return list(path.iterdir())
    except FileNotFoundError:
        return []


def assert_no_exact_resources(root, namespace, first_uid, firecracker_name, *, expected=None):
    """Observe only; never stop a host PID or delete a claim/resource directly."""
    _require_assertions(Failure)
    require(re.fullmatch(r"[0-9a-f]{16}", namespace) is not None, "identity_unknown")
    prefix = f"vm-{namespace}-"
    claims = entries(root / "jails/.claims")
    require(all(re.fullmatch(re.escape(prefix) + r"(0|[1-9][0-9]*)", p.name)
                and not p.is_symlink() and p.is_dir() for p in claims), "resource_unknown")
    require(len(claims) <= EXPECTED_SANDBOXES, "resource_unknown")
    network_inodes = set()
    identities = {}
    for evidence in entries(root / "resource-evidence"):
        require(not evidence.is_symlink() and evidence.is_file(), "resource_unknown")
        value = json_file(evidence)
        ident = value["sandbox"]
        require(valid_sandbox_id(ident, namespace) and evidence.name == ident + ".json", "identity_unknown")
        context, record, network = value["context"], value["record"], value["network"]
        require(context["binding"]["sandbox"] == int(ident.rsplit("-", 1)[1])
                and context["binding"]["owner"] == 1 and type(context["position"]) is int
                and 0 <= context["position"] < PHYSICAL_SLOTS
                and record["resource_id"] == prefix + str(context["binding"]["vm"]), "identity_unknown")
        require(type(record["pid"]) is int and 0 < record["pid"] < 2**32
                and type(record["start_time_ticks"]) is int and 0 < record["start_time_ticks"] < 2**64,
                "resource_unknown")
        require(process_start_ticks(record["pid"]) != record["start_time_ticks"], "resource_residual")
        require(network["resource_id"] == record["resource_id"] and type(network["namespace_inode"]) is int
                and network["namespace_inode"] > 0, "resource_unknown")
        require(record["resource_id"] not in identities, "identity_unknown")
        identities[record["resource_id"]] = record
        network_inodes.add(f"net:[{network['namespace_inode']}]")
    for claim in claims:
        try:
            record = json_file(claim / "record.json")
            require(isinstance(record, dict) and set(record) == {"resource_id", "pid", "start_time_ticks"}
                    and record["resource_id"] == claim.name
                    and type(record["pid"]) is int and 0 < record["pid"] < 2**32
                    and type(record["start_time_ticks"]) is int and 0 < record["start_time_ticks"] < 2**64,
                    "resource_unknown")
            require(process_start_ticks(record["pid"]) != record["start_time_ticks"], "resource_residual")
            require(claim.name not in identities or identities[claim.name] == record, "identity_unknown")
            identities[claim.name] = record
            if (claim / "network.json").exists():
                network = json_file(claim / "network.json")
                require(isinstance(network, dict) and set(network) == {"resource_id", "namespace_inode"}
                        and network["resource_id"] == claim.name and type(network["namespace_inode"]) is int
                        and network["namespace_inode"] > 0, "resource_unknown")
                network_inodes.add(f"net:[{network['namespace_inode']}]")
        except (OSError, ValueError, TypeError) as error:
            raise Failure("resource_unknown") from error
    for parent in (root / "jails" / firecracker_name, Path("/run/netns")):
        require(not any(p.name.startswith(prefix) for p in entries(parent)), "resource_residual")
    # os.walk's default swallows permission errors. An incomplete audit fails closed.
    def unavailable(error):
        raise Failure("resource_unknown") from error
    require(Path("/sys/fs/cgroup").is_dir(), "resource_unknown")
    for _directory, dirs, files in os.walk("/sys/fs/cgroup", onerror=unavailable):
        require(not any(name.startswith(prefix) for name in dirs + files), "resource_residual")
    require(not processes_in_uid_range(first_uid, first_uid + PHYSICAL_SLOTS - 1), "resource_residual")
    if network_inodes:
        for process in Path("/proc").iterdir():
            if process.name.isdecimal():
                try:
                    require(os.readlink(process / "ns/net") not in network_inodes, "resource_residual")
                except (FileNotFoundError, ProcessLookupError):
                    pass
    require(len(identities) <= EXPECTED_SANDBOXES, "resource_unknown")
    if expected is not None:
        require(len(identities) == expected, "resource_unknown")
    return len(identities)


def probe_environment(owner_key, foreign_key, evidence_root=None):
    # No inherited proxies, PYTHONOPTIMIZE, PYTHONPATH, user startup hooks or secrets.
    result = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8",
              "SANDPROOF_TEST_API_KEY": owner_key, "SANDPROOF_TEST_FOREIGN_API_KEY": foreign_key}
    if evidence_root is not None:
        result["SANDPROOF_TEST_EVIDENCE_ROOT"] = str(evidence_root)
    return result


def record_requests(root,services,report):
    """Parent-only private test callback to the ORIGINAL offline host audit."""
    _require_assertions(Failure)
    from _center_host_services import local_binding
    files=sorted((root/'resource-requests').iterdir())
    require(all(re.fullmatch(r'[0-9a-f]{64}\.(request|ack)\.json',path.name) is not None
                for path in files),'identity_unknown')
    requests=[path for path in files if path.name.endswith('.request.json')]
    require(len(requests)<=EXPECTED_SANDBOXES and len(files)<=2*EXPECTED_SANDBOXES,'identity_unknown')
    for path in requests:
        info=path.lstat()
        require(stat.S_ISREG(info.st_mode) and info.st_uid==0 and info.st_nlink==1
                and not info.st_mode&0o077 and info.st_size<=256,'identity_unknown')
        value=json_file(path,limit=256)
        require(type(value) is dict and set(value)=={'sandbox'},'identity_unknown')
        ident=value['sandbox']
        require(type(ident) is str and re.fullmatch(r'sandbox-[1-9][0-9]{0,19}',ident) is not None
                and int(ident.removeprefix('sandbox-'))<2**64,'identity_unknown')
        name=hashlib.sha256(ident.encode()).hexdigest()
        require(path.name==name+'.request.json','identity_unknown')
        ack=path.with_name(name+'.ack.json')
        if ack.exists():
            details=ack.lstat()
            require(stat.S_ISREG(details.st_mode) and details.st_uid==0 and details.st_nlink==1
                    and not details.st_mode&0o077 and details.st_size<=256,'identity_unknown')
            acknowledgement=json_file(ack,limit=256)
            require(type(acknowledgement) is dict and set(acknowledgement)=={'sandbox','recorded'}
                    and acknowledgement['sandbox']==ident and acknowledgement['recorded'] is True,'identity_unknown')
            continue
        center=services.planned_audit('center')
        matching=[row for row in center['sandboxes'] if row['create']['identity']['sandbox_id']==ident
                  and row['private'] is False and row['create']['identity']['tenant_id']=='1']
        require(len(matching)==1,'identity_unknown')
        identity=matching[0]['create']['identity']
        services.processes.stop('host-0')
        try:
            audit=services.processes.audit('host-0')
            local=local_binding(audit,ident,expected_node=services.host_configs[0]['inbox']['node'])
            require(local['owner']==1,'identity_unknown')
            number=local['sandbox']
            native_id='sb-'+services.native_pools[0]['namespace']+'-'+str(number)
            contexts=[context for context,_phase in audit['host_slots']
                      if type(context.get('binding')) is dict and context['binding']['sandbox']==number
                      and context['binding']['owner']==1]
            require(len(contexts)==1,'identity_unknown')
            context=contexts[0]
            name_vm='vm-'+services.native_pools[0]['namespace']+'-'+str(context['binding']['vm'])
            claim=root/'jails/.claims'/name_vm
            record,network=json_file(claim/'record.json'),json_file(claim/'network.json')
            require(record['resource_id']==name_vm and network['resource_id']==name_vm
                and process_start_ticks(record['pid'])==record['start_time_ticks'],'resource_unknown')
            uid=int(next(line.split()[1] for line in Path('/proc',str(record['pid']),'status').read_text().splitlines()
                         if line.startswith('Uid:')))
            require(uid==services.native_pools[0]['bounded_host']['uid_base']+context['position'],'resource_unknown')
            write_once(root/'resource-evidence'/(native_id+'.json'),dict(sandbox=native_id,global_id=ident,
                create_identity=identity,context=context,record=record,network=network))
            report.setdefault('global_identities',{})[ident]=identity
            report.setdefault('global_native',{})[ident]=native_id
        finally:
            services.processes.start('host-0')
        services.wait_ready(for_admission=False)
        write_once(ack,dict(sandbox=ident,recorded=True))


def run_probe(root, name, base_url, environment, namespace, *, timeout=240,services=None,report=None):
    _require_assertions(Failure)
    require(name in PROBES, "invalid_configuration")
    process = None
    primary = None
    secondary = []
    with (root / f"{name}.stdout").open("xb") as out, (root / f"{name}.stderr").open("xb") as err:
        try:
            process = subprocess.Popen(
                [sys.executable, "-E", "-s", str(CHECKOUT / "tests/system" / name),
                 "--api-url", base_url, "--sandbox-url", base_url],
                stdin=subprocess.DEVNULL, stdout=out, stderr=err, env=environment,
                start_new_session=True,
            )
            try:
                if services is None:
                    code = process.wait(timeout=timeout)  # Inert component test.
                else:
                    deadline=time.monotonic()+timeout
                    while True:
                        record_requests(root,services,report)
                        left=deadline-time.monotonic()
                        if left<=0:
                            raise Failure('timeout')
                        try:
                            code=process.wait(timeout=min(.1,left))
                            break
                        except subprocess.TimeoutExpired:
                            continue
            except subprocess.TimeoutExpired as error:
                raise Failure("timeout") from error
            require(code == 0, "probe_failed")
        except BaseException as error:
            primary = error
        finally:
            # The caller cannot discover/clean DB identities until this child is reaped.
            try:
                stop_child(process)
            except BaseException as error:
                secondary.append(error)
            for output in (out, err):
                try:
                    output.flush()
                    os.fsync(output.fileno())
                except BaseException as error:
                    secondary.append(error)
        if primary is not None or secondary:
            failure = primary if primary is not None else secondary.pop(0)
            failure.secondary = secondary
            failure.child_reaped = process is None or process.poll() is not None
            raise failure
    try:
        value = json_file(root / f"{name}.stdout")
        return validate_probe(name, value, namespace)
    except (OSError, ValueError, TypeError, KeyError) as error:
        raise Failure("probe_output_invalid") from error


def preflight(args, report):
    _require_assertions(Failure)
    require(re.fullmatch(r"[0-9a-f]{40}", args.expected_commit or "") is not None,
            "invalid_configuration")
    require(sys.platform == "linux" and os.geteuid() == 0, "preflight_failed")
    require(1000 <= args.uid_base and args.uid_base + PHYSICAL_SLOTS - 1 < 2**32 - 1, "invalid_configuration")
    require(importlib.metadata.version("e2b") == "2.51.0", "sdk_mismatch")
    source = host.source_revision()
    require(source["revision"] == args.expected_commit and not source["dirty"], "source_mismatch")
    report["commit"] = source["revision"]
    config, config_digest = host.load_config(host.trusted_path(args.host_config))
    report["hashes"]["host_config"] = config_digest
    # Match retained old flags to the one existing approved host profile.
    for name in host.ARTIFACTS:
        path = host.trusted_path(getattr(args, name))
        require(path == Path(config["artifacts"][name]["path"]), "invalid_configuration")
        observed = digest(path)
        require(observed == config["artifacts"][name]["sha256"], "hash_mismatch")
        report["hashes"][name] = observed
    paths={name:host.trusted_path(getattr(args,name)) for name in ('center','host')}
    receipt=host.trusted_path(args.candidate_manifest)
    versions=json_file(receipt,limit=16384)['versions']
    manifest=artifacts.load(receipt,candidate=args.expected_commit,paths=paths,versions=versions)
    report['services']=artifacts.public_identity(manifest)
    for name,path in paths.items():
        require(os.access(path,os.X_OK),'hash_mismatch')
        report['hashes'][name]=digest(path)
    report['hashes']['candidate_manifest']=digest(args.candidate_manifest)
    report["hashes"]["python"] = digest(Path(sys.executable).resolve(strict=True))
    for name, relative in SOURCE_FILES.items():
        path = CHECKOUT / relative
        require(not path.is_symlink(), "source_mismatch")
        report["hashes"][name] = digest(path)
        # Also catch assume-unchanged index entries; bind probe bytes to HEAD.
        committed = subprocess.check_output(
            ["git", "--no-optional-locks", "-c", f"safe.directory={CHECKOUT}",
             "-C", str(CHECKOUT), "show", f"{args.expected_commit}:{relative}"], timeout=10)
        require(hashlib.sha256(committed).hexdigest() == report["hashes"][name], "source_mismatch")
    # Only after hashes/clean commit are checked may approved host binaries run
    # their --version probes. This is the existing read-only preflight, not KVM acceptance.
    result = host.inspect_host(config, config_digest)
    report["private_preflight"] = result
    report["preflight"] = {item["name"]: item["status"] for item in result["checks"]}
    require(result["prerequisites_passed"] and
            all(value == "pass" for value in report["preflight"].values()), "preflight_failed")
    require(not processes_in_uid_range(args.uid_base, args.uid_base + PHYSICAL_SLOTS - 1), "preflight_failed")
    require(shutil.disk_usage(args.scratch_parent).free >= args.rootfs.stat().st_size * 3 + 256 * 1024 * 1024,
            "preflight_failed")


def prepare(args, root, namespace, owner_key, foreign_key, port, report):
    _require_assertions(Failure)
    from server_templates import descriptor
    firecracker = args.firecracker.name
    longest_socket = root / "jails" / firecracker / f"vm-{namespace}-{2**64 - 1}" / "root/api.socket"
    require(len(os.fsencode(longest_socket)) < 108, "invalid_configuration")
    for name in ("db", "state", "jails","resource-evidence","resource-requests"):
        (root / name).mkdir(mode=0o700)
    report["private_staged"] = {}
    report["staged_copies"] = "retained"
    for name, source, destination in (("kernel", args.kernel, root / "vmlinux"),
                                     ("rootfs", args.rootfs, root / "rootfs.ext4")):
        report["staged_files"][name] = "unknown"
        shutil.copyfile(source, destination)
        destination.chmod(0o400)
        require(digest(destination) == report["hashes"][name], "hash_mismatch")
        metadata = destination.lstat()
        report["private_staged"][name] = {"device": metadata.st_dev, "inode": metadata.st_ino,
            "bytes": metadata.st_size,'owner':metadata.st_uid,'path':str(destination),
            'parent_device':destination.parent.stat().st_dev,'parent_inode':destination.parent.stat().st_ino}
        report["staged_files"][name] = "retained"

    disk_bytes = args.rootfs.stat().st_size
    config = {
        "namespace": namespace, "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"), "jailer": str(args.jailer),
        "firecracker": str(args.firecracker), "chroot_base": str(root / "jails"),
        "kernel": str(root / "vmlinux"), "rootfs": str(root / "rootfs.ext4"),
        "uid_base": args.uid_base, "vcpu_count": 1,
        "memory_mib": 512, "file_policy_digest": 1,
        "max_active_global": 2, "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 6, "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes, "max_timeout_secs": 600,
        "max_lifetime_secs": 600, "default_timeout_secs": 300, "expiry_scan_seconds": 1,
    }
    config = complete_runtime(config, slots=PHYSICAL_SLOTS)
    # The fixed smoke's 8 MiB+1 refusal and data-contract corpus share this
    # explicit fixture upload budget; it is not an SDK or product default.
    config["data_transfer"]["file_upload_bytes"] = 8 * 1024 * 1024
    provision_runtime(config)
    binding=descriptor('base',root/'vmlinux',root/'rootfs.ext4',1,512)
    catalog=dict(templates=[binding],aliases={'base':'base'},compute_limits=dict(
        max_vcpus_global=2,max_vcpus_per_tenant=2,max_memory_bytes_global=2*768*1024**2,
        max_memory_bytes_per_tenant=2*768*1024**2))
    config['templates']=catalog
    services=RuntimeServices.prepare(args,root,[config],[catalog],owner_key,f'127.0.0.1:{port}',
        args.service_cgroup_record,job_count=EXPECTED_SANDBOXES,extra_credentials=[dict(tenant=2,secret=foreign_key)],
        registry_config=dict(root=str(root/'unused-registry'),max_templates_global=1,max_templates_per_tenant=1,
            max_artifact_bytes_global=disk_bytes+args.kernel.stat().st_size,
            max_artifact_bytes_per_tenant=disk_bytes+args.kernel.stat().st_size,validation_timeout_secs=60))
    for name in ('center','host'):
        role=services.processes.bindings['host-0' if name=='host' else name]
        path=Path(role['binary'])
        info=path.lstat()
        report['private_staged'][name]=dict(device=info.st_dev,inode=info.st_ino,bytes=info.st_size,
            owner=role['uid'],path=str(path),parent_device=path.parent.stat().st_dev,
            parent_inode=path.parent.stat().st_ino)
        report['staged_files'][name]='retained'
    budget=root/'service-budgets.json'
    write_once(budget,services.native_pools[0]['bounded_host']['physical_storage'])
    config_receipt={name:digest(Path(role['config'])) for name,role in services.processes.bindings.items()}
    report['hashes']['service_config']=hashlib.sha256(json.dumps(config_receipt,sort_keys=True,
        separators=(',',':')).encode()).hexdigest()
    return services


def kill_one(opener, base_url, owner_key, ident):
    _require_assertions(Failure)
    request = urllib.request.Request(base_url + "/sandboxes/" + ident,
                                     headers={"X-API-Key": owner_key}, method="DELETE")
    try:
        with opener.open(request, timeout=5) as response:
            status = response.status
    except urllib.error.HTTPError as error:
        status = error.code
        error.close()
    require(status in (204, 404), "cleanup_failed")


def cleanup(root, namespace, args, opener, base_url, owner_key, services, report):
    _require_assertions(Failure)
    """Attempt every observed exact persistent identity even after another fails."""
    report["cleanup"] = "unknown"
    require(report.get("probe_reaped", True), "identity_unknown")
    observed = dict(report.get('global_identities',{}))
    attempted = set()
    deadline=time.monotonic()+30
    while True:
        center=services.planned_audit('center')
        records={row['create']['identity']['sandbox_id']:row for row in center['sandboxes']
                 if row['private'] is False and row['create']['identity']['tenant_id']=='1'}
        require(len(records)<=EXPECTED_SANDBOXES,'identity_unknown')
        for ident,row in records.items():
            identity=row['create']['identity']
            require(ident not in observed or observed[ident]==identity,'identity_unknown')
            observed[ident]=identity
        report['global_identities']=dict(observed)
        report['known_sandboxes']=len(observed)
        # Original terminal confirmation is independent of HTTP 204/404 and
        # native SQL retirement. Never resend an already dispatched Destroy.
        host_audit=services.planned_audit('host-0')
        terminals={ident:terminal_result(center,host_audit,identity)
                   for ident,identity in observed.items()}
        for ident in sorted(observed):
            if terminals[ident] is not None or ident in attempted:
                continue
            row=records.get(ident)
            if row is None:
                continue  # Missing original record is UNKNOWN, not Cleaned.
            latest=row['operations'][-1]['intent'] if row['operations'] else None
            if latest is not None and latest['kind']=='destroy':
                attempted.add(ident)
                continue
            attempted.add(ident)  # Mark BEFORE the one original HTTP request.
            try:
                services.wait_ready(for_admission=False)
                kill_one(opener,base_url,owner_key,ident)
            except Exception as error:
                record_failure(report,'cleanup',error,cleanup=True)
        if all(terminals.values()):
            break
        require(time.monotonic()<deadline,'terminal_state_invalid')
        time.sleep(.2)
    native=dict(report.get('global_native',{}))
    require(set(native)==set(observed),'identity_unknown')
    report['identities']=sorted(native.values())
    wait_for_destroyed(root / 'db/controller.db',namespace,native.values())
    report["known_claims"] = assert_no_exact_resources(
        root, namespace, args.uid_base, args.firecracker.name)
    require(all(child.poll() is None for child in services.processes.children.values()),'server_exited')
    require(not report["cleanup_errors"], "cleanup_failed")
    if len(observed) == EXPECTED_SANDBOXES:
        require(report["known_claims"] == EXPECTED_SANDBOXES, "resource_unknown")
    # No drain API exists for an in-flight lost create; a partial failed probe
    # cannot prove that every absent profile instance was never dispatched.
    require(not report["failures"] or len(observed) == EXPECTED_SANDBOXES, "identity_unknown")
    report["cleanup"] = "confirmed"
    report["phases"]["cleanup"] = "pass"
    return set(native.values())


def unchanged(args, root, report):
    _require_assertions(Failure)
    for name in ("center", "host", "candidate_manifest", "kernel", "rootfs", "jailer", "firecracker"):
        require(digest(getattr(args, name)) == report["hashes"][name], "hash_mismatch")
    require(digest(root / "rootfs.ext4") == report["hashes"]["rootfs"], "hash_mismatch")
    for name, path in SOURCE_FILES.items():
        require(digest(CHECKOUT / path) == report["hashes"][name], "hash_mismatch")
    require(host.source_revision()["revision"] == args.expected_commit, "source_mismatch")


def remove_staged_copies(root, report):
    _require_assertions(Failure)
    """After successful shutdown/audit, unlink only the four registered copies."""
    report["staged_copies"] = "unknown"
    files=tuple((name,Path(report['private_staged'][name]['path'])) for name in
                ('center','host','kernel','rootfs'))
    try:
        with ExitStack() as opened:
            validated = {}
            # Validate the complete set before removing any file. Retain all
            # available copies when one target has changed or cannot be observed.
            directories={}
            for name, path in files:
                report["staged_files"][name] = "unknown"
                expected = report["private_staged"][name]
                directory=os.open(path.parent,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
                opened.callback(os.close,directory)
                parent=os.fstat(directory)
                expected_name={'center':'sandproof-center','host':'sandproof-host',
                    'kernel':'vmlinux','rootfs':'rootfs.ext4'}[name]
                require(path.name==expected_name and path.resolve(strict=True)==path
                        and parent.st_uid==expected['owner'] and not parent.st_mode&0o077
                        and (parent.st_dev,parent.st_ino)==(expected['parent_device'],expected['parent_inode']),
                        'identity_unknown')
                directories[name]=directory
                fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
                source = opened.enter_context(os.fdopen(fd, "rb"))
                info = os.fstat(source.fileno())
                require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1
                        and info.st_uid == expected['owner']
                        and (info.st_dev, info.st_ino, info.st_size) ==
                        (expected["device"], expected["inode"], expected["bytes"]), "identity_unknown")
                require(hashlib.file_digest(source, "sha256").hexdigest() == report["hashes"][name],
                        "hash_mismatch")
                validated[name] = info
                report["staged_files"][name] = "retained"
            for name, path in files:
                info = validated[name]
                directory=directories[name]
                current = os.stat(path.name, dir_fd=directory, follow_symlinks=False)
                require(stat.S_ISREG(current.st_mode) and current.st_nlink == 1
                        and (current.st_dev, current.st_ino, current.st_size) ==
                        (info.st_dev, info.st_ino, info.st_size), "identity_unknown")
                report["staged_files"][name] = "unknown"
                os.unlink(path.name, dir_fd=directory)
                report["staged_files"][name] = "removed"
            for directory in directories.values():
                os.fsync(directory)
    except BaseException:
        raise
    report["staged_copies"] = "removed"


def run_profile(args, root, report):
    _require_assertions(Failure)
    services = None
    started = False
    identities = None
    namespace = report["private_identity"]["namespace"]
    owner_key, foreign_key = secrets.token_hex(32), secrets.token_hex(32)
    base_url = None
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    stage = "preflight"
    try:
        preflight(args, report)
        report["phases"][stage] = "pass"
        stage = "prepare"
        port = free_loopback_port()
        base_url = f"http://127.0.0.1:{port}"
        services=prepare(args, root, namespace, owner_key, foreign_key, port, report)
        report["phases"][stage] = "pass"
        # Durable provenance before launching any service that can create VMs.
        write_once(root / "ready.json", report)
        stage = "server_start"
        started = True
        report["cleanup"] = "unknown"
        services.start()
        report["kvm"] = "attempted"
        services.wait_ready()
        run_identity=hashlib.sha256((report['run_id']+report['commit']).encode()).hexdigest()
        execution,processes=services.execution(run_identity,root/'service-budgets.json')
        write_once(root/'execution-manifest.json',execution)
        report['execution_manifest']=execution
        report['private_candidate_manifest']=services.manifest
        report['service_processes']=processes
        report['hashes']['execution_manifest']=digest(root/'execution-manifest.json')
        report['services']=execution['services']
        report['service_boundary']=dict(public_api='center',center_unprivileged=True,
            redis_authority='validated',host_transport='mutual-tls',host_public_management=False)
        report["phases"][stage] = "pass"
        for name in PROBES:
            stage = name
            report["probes"][name] = run_probe(
                root, name, base_url, probe_environment(owner_key, foreign_key, root), namespace,
                services=services,report=report)
            report["kvm"] = "executed"
            require(all(child.poll() is None for child in services.processes.children.values()),'server_exited')
            report["phases"][stage] = "pass"
        stage = "terminal_state"
        rows = durable_rows(root / "db/controller.db", namespace)
        report["known_sandboxes"] = len(rows)
        report["identities"] = sorted(rows)
        require(len(rows) == EXPECTED_SANDBOXES, "terminal_state_invalid")
        require(set(report.get('global_identities',{}))==set(report.get('global_native',{}))
                and len(report.get('global_identities',{}))==EXPECTED_SANDBOXES,'identity_unknown')
        wait_for_destroyed(root / "db/controller.db", namespace, rows)
        report["phases"][stage] = "pass"
        stage = "resource_audit"
        report["known_claims"] = assert_no_exact_resources(
            root, namespace, args.uid_base, args.firecracker.name, expected=EXPECTED_SANDBOXES)
        report["phases"][stage] = "pass"
        stage = "source_unchanged"
        unchanged(args, root, report)
        report["phases"][stage] = "pass"
    except BaseException as error:
        record_failure(report, stage, error)
        report["probe_reaped"] = getattr(error, "child_reaped", True)
    finally:
        # Repeated SIGINT/TERM cannot abort exact cleanup or replace the first error.
        for sig in (signal.SIGINT, signal.SIGTERM):
            signal.signal(sig, signal.SIG_IGN)
        try:
            if started:
                try:
                    identities = cleanup(root, namespace, args, opener, base_url, owner_key, services, report)
                except BaseException as error:
                    record_failure(report, "cleanup", error, cleanup=True)
                    report["cleanup"] = "unknown"
                    identities = set(report.get("identities", [])) if "identities" in report else None
            else:
                report["cleanup"] = "not_created"
        finally:
            try:
                if services is not None:
                    services.stop_all()
                report["phases"]["server_stop"] = "pass"
            except BaseException as error:
                record_failure(report, "server_stop", error, cleanup=True)
                report["cleanup"] = "unknown"
        # Catch a late accepted request between the cleanup snapshot and shutdown.
        # Never restart/replay automatically when the now-stopped service cannot help.
        if identities is not None:
            try:
                rows = durable_rows(root / "db/controller.db", namespace)
                require(set(rows) == identities and destroyed(rows), "identity_unknown")
                assert_no_exact_resources(root, namespace, args.uid_base, args.firecracker.name)
            except BaseException as error:
                record_failure(report, "cleanup", error, cleanup=True)
                report["cleanup"] = "unknown"
    report["status"] = "interrupted" if any(
        item["code"] == "interrupted" for item in report["failures"]) else "fail"
    if not report["failures"] and not report["cleanup_errors"]:
        try:
            remove_staged_copies(root, report)
            require(services is not None,'identity_unknown')
            services.cleanup_staging()
            report["phases"]["artifact_cleanup"] = "pass"
            report["status"] = "pass"
        except BaseException as error:
            record_failure(report, "artifact_cleanup", error)
            report["status"] = "fail"


def exercise(args):
    _require_assertions(Failure)
    root = None
    lock = None
    report = new_report("unknown")
    original_signals = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
    old_umask = os.umask(0o077)
    def interrupted(_signal, _frame):
        raise Failure("interrupted")
    try:
        for sig in original_signals:
            signal.signal(sig, interrupted)
        # Root creation itself is unprivileged filesystem work. Invalid preflight
        # still gets a terminal report whenever a safe private parent is available.
        parent = args.scratch_parent
        require(parent.is_absolute() and not parent.is_symlink(), "invalid_configuration")
        parent = host.trusted_path(parent)
        require(parent.is_dir() and not parent.stat().st_mode & 0o077, "invalid_configuration")
        root = Path(tempfile.mkdtemp(prefix="sp12-", dir=parent))
        lock = os.open(root / "run.lock", os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        report = new_report(root.name)
        report["private_identity"] = {"namespace": secrets.token_hex(8),
                                      "uid_first": args.uid_base, "uid_last": args.uid_base + PHYSICAL_SLOTS - 1}
        write_once(root / "intent.json", report)
        run_profile(args, root, report)
    except BaseException as error:
        record_failure(report, "preflight", error)
        report["status"] = "interrupted" if isinstance(error, Failure) and error.code == "interrupted" else "fail"
    finally:
        for sig in original_signals:
            signal.signal(sig, signal.SIG_IGN)
        if root is not None:
            try:
                report["phases"]["report"] = "pass"
                write_once(root / "terminal.json", report)
            except BaseException as error:
                failure = Failure("report_write_failed")
                failure.secondary = getattr(error, "secondary", [])
                record_failure(report, "report", failure)
                report["status"] = "fail"
                if getattr(error, "publication_unknown", False):
                    # Best effort only: a filesystem that cannot record even this
                    # marker requires operator review, never acceptance from files.
                    try:
                        write_once(root / "publication-unknown.json", {"code": "report_write_failed"})
                    except BaseException:
                        pass
    exported = safe_export(report)
    try:
        print(json.dumps(exported, indent=2), flush=True)
    except OSError:
        if root is not None:
            try:
                write_once(root / "delivery-failed.json", {"code": "export_write_failed"})
            except OSError:
                pass
        return 1
    finally:
        if lock is not None:
            os.close(lock)
        for sig, handler in original_signals.items():
            signal.signal(sig, handler)
        os.umask(old_umask)
    return 0 if exported["status"] == "pass" else 1


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--export-run", type=Path, help="read-only safe summary; no resume or cleanup")
    for name in ("center","host","candidate-manifest","redis-material","tls-material",
                 "service-staging-parent","service-cgroup-record","jailer", "firecracker", "kernel", "rootfs", "scratch-parent", "host-config"):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--uid-base", type=int)
    parser.add_argument("--expected-commit")
    for name in ('center-uid','center-gid','redis-uid','redis-gid'):
        parser.add_argument('--'+name,type=int)
    args = parser.parse_args(argv)
    if args.export_run is not None:
        try:
            result = read_report(args.export_run)
            print(json.dumps(result, indent=2))
            return 0 if result["status"] == "pass" else 1
        except (OSError, ValueError, TypeError, Failure):
            print(json.dumps({"schema_version": 2, "profile": "c02", "status": "fail",
                              "cleanup": "unknown", "error": "report_unavailable"}))
            return 1
    required = ("center","host","candidate_manifest","redis_material","tls_material","service_staging_parent",
                "service_cgroup_record","center_uid","center_gid","redis_uid","redis_gid", "jailer", "firecracker", "kernel", "rootfs", "scratch_parent", "host_config",
                "uid_base", "expected_commit")
    if any(getattr(args, name) is None for name in required):
        parser.error("execution requires the exact center/host manifest, fixed Redis/TLS/role inputs and approved host profile")
    return exercise(args)


if __name__ == "__main__":
    sys.exit(main())
