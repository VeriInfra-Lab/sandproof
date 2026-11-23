"""Bind native sandbox IDs to live VMM claims before host window retirement."""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


from pathlib import Path


def capture_live_native_ids(native, claims, read_slot):
    namespace = native["namespace"]
    count = native["bounded_host"]["maintenance"]["max_host_slots"]
    resources = {claim["resource_id"] for claim in claims}
    if type(count) is not int or count <= 0 or len(claims) != count or len(resources) != count:
        raise ValueError("complete distinct original VM claims required")
    directory = Path(native["node_state_dir"]) / "host-slots"
    sandbox_ids = set()
    for position in range(count):
        row = read_slot(directory / f"{position}.json", 65536)
        value = row["value"]
        if row["position"] != position or value["phase"] != "Occupied":
            raise ValueError("native sandbox identity must be observed in its occupied slot")
        context = value["context"]
        binding = context["binding"]
        sandbox, vm = binding["sandbox"], binding["vm"]
        if (context["position"] != position or type(sandbox) is not int or sandbox <= 0
                or type(vm) is not int or vm <= 0 or sandbox in sandbox_ids):
            raise ValueError("slot does not identify a distinct original native sandbox")
        resource = f"vm-{namespace}-{vm}"
        if resource not in resources:
            raise ValueError("occupied slot does not match an original VM claim")
        resources.remove(resource)
        sandbox_ids.add(sandbox)
    return tuple(f"sb-{namespace}-{sandbox}" for sandbox in sorted(sandbox_ids))
