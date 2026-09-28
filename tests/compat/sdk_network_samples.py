"""Golden network/detail samples for the pinned SDK and real service probes.

These are fixed inputs and expected fields, not a network-policy evaluator.
Only the real probe checks packet reachability and restart persistence.
"""

from dataclasses import dataclass


@dataclass(frozen=True)
class NetworkSample:
    name: str
    options: dict
    internet: bool | None
    sdk_network: dict | None
    wire_network: dict | None
    reachable: tuple[bool, bool] | None = None


PROFILES = (
    NetworkSample('omitted_public', {}, None, None, None, (True, True)),
    NetworkSample(
        'false_with_allow',
        {'allow_internet_access': False,
         'network': {'allow_out': ['11.22.33.7/24', '11.22.33.0/24'],
                     'deny_out': ['0.0.0.0/0']}},
        False,
        {'allow_public_traffic': True, 'allow_out': ['11.22.33.7/24', '11.22.33.0/24'],
         'deny_out': ['0.0.0.0/0']},
        {'allowPublicTraffic': True, 'allowOut': ['11.22.33.7/24', '11.22.33.0/24'],
         'denyOut': ['0.0.0.0/0']},
        (True, False)),
    NetworkSample('explicit_public', {'allow_internet_access': True},
                  True, None, None, (True, True)),
    NetworkSample('explicit_closed', {'allow_internet_access': False},
                  False, None, None, (False, False)),
    NetworkSample(
        'public_with_deny',
        {'allow_internet_access': True,
         'network': {'allow_out': [], 'deny_out': ['11.22.34.1']}},
        True,
        {'allow_public_traffic': True, 'allow_out': [], 'deny_out': ['11.22.34.1']},
        {'allowPublicTraffic': True, 'allowOut': [], 'denyOut': ['11.22.34.1']},
        (True, False)),
    NetworkSample(
        'allow_precedes_deny',
        {'network': {'allow_out': ['0.0.0.0/0'], 'deny_out': ['0.0.0.0/0']}},
        None,
        {'allow_public_traffic': True, 'allow_out': ['0.0.0.0/0'], 'deny_out': ['0.0.0.0/0']},
        {'allowPublicTraffic': True, 'allowOut': ['0.0.0.0/0'], 'denyOut': ['0.0.0.0/0']},
        (True, True)),
)

DETAIL_NETWORK = NetworkSample(
    'detail_static',
    {'allow_internet_access': False,
     'network': {'allow_out': [], 'deny_out': ['8.8.8.8', '1.2.3.4/24']}},
    False,
    {'allow_public_traffic': True, 'allow_out': [], 'deny_out': ['8.8.8.8', '1.2.3.4/24']},
    {'allowPublicTraffic': True, 'allowOut': [], 'denyOut': ['8.8.8.8', '1.2.3.4/24']})
DETAIL_PROFILES = (PROFILES[3], DETAIL_NETWORK)
DETAIL_FIELDS = {
    'cpuCount': 1, 'memoryMB': 512, 'diskSizeMB': 1024, 'volumeMounts': [],
    'lifecycle': {'onTimeout': 'kill', 'autoResume': False},
}


def validate_network(info, raw, sample):
    assert info.allow_internet_access is sample.internet, sample.name
    assert info.network == sample.sdk_network, sample.name
    assert raw['allowInternetAccess'] is sample.internet, sample.name
    if sample.wire_network is None:
        assert 'network' not in raw, sample.name
    else:
        assert raw['network'] == sample.wire_network, sample.name


def validate_base_details(info, raw, metadata):
    assert info.sandbox_domain == 'example.invalid'
    assert info.lifecycle == {'on_timeout': 'kill', 'auto_resume': False}
    assert info.volume_mounts == [] and info.name is None
    assert info.metadata == metadata
    assert info.cpu_count == 1 and info.memory_mb == 512
    assert raw['envdAccessToken'] and 'alias' not in raw
    for field, expected in DETAIL_FIELDS.items():
        assert raw[field] == expected, field
