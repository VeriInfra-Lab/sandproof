"""Record exact real-VM identities before a C02 child can destroy or expire them.

The private C02 supervisor supplies its owned directory explicitly. Other SDK
probes do no host I/O when that variable is absent. These are transient test
observations, not product tombstones or worker success receipts.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

from contextlib import closing
import json
import os
from pathlib import Path
import re
import sqlite3
import stat


def record_resource_identity(sandbox):
    configured = os.environ.get("SANDPROOF_TEST_EVIDENCE_ROOT")
    if configured is None:
        return
    root = Path(configured)
    metadata = root.lstat()
    if (not root.is_absolute() or not stat.S_ISDIR(metadata.st_mode)
            or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077):
        raise RuntimeError("private owned C02 evidence root required")
    settings = json.loads((root / "config.json").read_text())
    namespace = settings["namespace"]
    if re.fullmatch(r"sb-" + re.escape(namespace) + r"-[1-9][0-9]*", sandbox) is None:
        raise RuntimeError("sandbox outside the private C02 deployment")
    number = int(sandbox.rsplit("-", 1)[1])
    with closing(sqlite3.connect(Path(settings["database"]).as_uri() + "?mode=ro", uri=True)) as connection:
        connection.execute("BEGIN")
        contexts = [json.loads(row[0]) for row in connection.execute("SELECT context_json FROM controller_host_slots")]
    matches = [value for value in contexts if value["binding"]["sandbox"] == number and value["binding"]["owner"] == 1]
    if len(matches) != 1:
        raise RuntimeError("sandbox has no exact current physical binding")
    context = matches[0]
    name = f"vm-{namespace}-{context['binding']['vm']}"
    claim = Path(settings["chroot_base"]) / ".claims" / name
    record = json.loads((claim / "record.json").read_text())
    network = json.loads((claim / "network.json").read_text())
    if (set(record) != {"resource_id", "pid", "start_time_ticks"} or record["resource_id"] != name
            or type(record["pid"]) is not int or record["pid"] <= 0
            or type(record["start_time_ticks"]) is not int or record["start_time_ticks"] <= 0
            or network["resource_id"] != name or type(network["namespace_inode"]) is not int
            or network["namespace_inode"] <= 0):
        raise RuntimeError("malformed exact C02 VM evidence")
    fields = Path(f"/proc/{record['pid']}/stat").read_text().rsplit(")", 1)[1].split()
    uid = int(next(line.split()[1] for line in Path(f"/proc/{record['pid']}/status").read_text().splitlines()
                   if line.startswith("Uid:")))
    if (fields[0] == "Z" or int(fields[19]) != record["start_time_ticks"]
            or uid != settings["bounded_host"]["uid_base"] + context["position"]):
        raise RuntimeError("live C02 VM identity/physical UID differs")
    evidence = root / "resource-evidence"
    metadata = evidence.lstat()
    if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077):
        raise RuntimeError("private resource-evidence directory required")
    descriptor = os.open(evidence / (sandbox + ".json"), os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump({"sandbox": sandbox, "context": context, "record": record, "network": network}, output)
        output.flush()
        os.fsync(output.fileno())
    directory = os.open(evidence, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)
