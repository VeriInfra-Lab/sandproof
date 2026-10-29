"""Prepare only private native views; ordinary execution belongs to ning."""
from pathlib import Path
import hashlib
import json

HERE = Path(__file__).resolve().parent
SOURCE = "controller-service/src/controller/lifecycle_inbox.rs"
FAILURES = {
    "wrong-owner-guard": ["proof_owner_full_external_match", "proof_owner_world_maximum",
                          "proof_owner_high_water"],
    "wrong-world-maximum": ["proof_owner_world_maximum", "proof_owner_world_overflow",
                            "proof_owner_boundary_reaches_max"],
    "wrong-high-water": ["proof_owner_high_water", "proof_owner_high_water_overflow",
                         "proof_owner_boundary_reaches_max"],
    "wrong-overflow": ["proof_owner_high_water_overflow", "proof_owner_world_overflow"],
}
VARIANTS = ["original", "candidate", *FAILURES, "restored"]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError("unknown native view")
    inv = json.loads((HERE / "inventory.json").read_text())
    path = project / SOURCE
    if sha(path) != inv["source_inputs"][SOURCE]:
        raise ValueError("original production source differs")
    original = (HERE / "original-owner-for.rs.in").read_bytes()
    candidate = (HERE / "candidate-owner-for.rs.in").read_bytes()
    if (hashlib.sha256(original).hexdigest() != inv["candidate_original_body_sha256"]
            or hashlib.sha256(candidate).hexdigest() != inv["candidate_body_sha256"]):
        raise ValueError("reviewed body differs")
    data = path.read_bytes()
    if data.count(original) != 1:
        raise ValueError("original body is not unique")
    mutation = None
    if variant not in ("original", "restored"):
        replacement = candidate
        if variant in FAILURES:
            mutation = inv["variants"][variant]["mutation"]
            before, after = mutation["before"].encode(), mutation["after"].encode()
            if replacement.count(before) != 1:
                raise ValueError("real mutation is not unique")
            replacement = replacement.replace(before, after, 1)
        data = data.replace(original, replacement, 1)
        path.write_bytes(data)
    return {SOURCE: sha(path)}, dict(variant=variant, mutation=mutation,
        original_source_sha256=inv["source_inputs"][SOURCE], derived_source_sha256=sha(path),
        production_modified=False, original_equivalence_proved=False)
