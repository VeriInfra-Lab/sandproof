"""Finite summary parser fixture only; never an executed service/Redis claim."""

from tools.service_storage_observation import LIMITATIONS, OBSERVATION


def observation():
    return dict(schema_version=1, observation=OBSERVATION, limitations=LIMITATIONS,
        sample_interval_ms=50, successful_samples=2, unknown_samples=1, sampler_exited=True,
        roots=[dict(index=0, identity=[1, 2], owner_uid=0,
            budget=dict(logical_file_bytes=1024, entries=4),
            observed_peak=dict(logical_file_bytes=41, allocated_bytes=16384, entries=4),
            last_stable=dict(logical_file_bytes=24, allocated_bytes=12288, entries=3))])
