"""Review the current isolated Redis diagnostic connection, not equivalence.

This bounded source admission supplements complete LLBC/Drop and input hashes.
It never turns logging, health policy or general original/candidate equivalence
into a proved fact. Run against the actual prepared candidate before extraction.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
STORE = 'sandbox-center-store/src/store.rs'
DIAGNOSTICS = 'sandbox-center-store/src/store/diagnostics.rs'
IO = 'sandbox-center-store/src/store/storage_io.rs'


def compact(text):
    return re.sub(r'\s+', '', text)


def function(text, name):
    matches = list(re.finditer(r'\bfn ' + re.escape(name) + r'(?:<[^\n]+>)?\s*\(', text))
    if len(matches) != 1:
        raise ValueError('missing or duplicate reviewed function: ' + name)
    start = text.index('{', matches[0].end())
    depth, stop = 1, start + 1
    while depth and stop < len(text):
        depth += (text[stop] == '{') - (text[stop] == '}')
        stop += 1
    if depth:
        raise ValueError('unbalanced reviewed function: ' + name)
    return text[start + 1:stop - 1]


def check(root, candidate):
    original = (root / STORE).read_text()
    source = (candidate / STORE).read_text()
    io = (candidate / IO).read_text()
    profile = json.loads((root / 'verification/experiments/redis-result/candidate.json').read_text())
    checked = []

    def require(name, condition):
        if not condition:
            raise ValueError('Redis diagnostic connection changed: ' + name)
        checked.append(name)

    require('current original store', hashlib.sha256((root / STORE).read_bytes()).hexdigest() == profile['original_store_sha256'])
    require('original diagnostic implementation', (root / DIAGNOSTICS).read_bytes() == (candidate / DIAGNOSTICS).read_bytes())
    require('zero-resource endpoint', len(re.findall(r'^pub\(super\) struct Diagnostics;$', io, re.M)) == 1)
    require('one original output call and no business access', compact(function(io, 'emit')) == 'diagnostics::emit(phase,failure,elapsed,budget);')
    signature = 'pub(super) fn emit(_output: &mut Diagnostics, phase: Phase, failure: Failure, elapsed: Duration, budget: Duration,)'
    actual = io[io.index('pub(super) fn emit('):io.index('{', io.index('pub(super) fn emit('))]
    require('only endpoint and original metadata inputs', compact(actual) == compact(signature))
    query = function(original, 'query').replace('self.connection.as_mut()', 'connection.as_mut()').replace('self.connection = None', '*connection = None')
    require('complete moved query body', compact(query) == compact(function(io, 'query')))
    require('connection discarded before original diagnostic', compact('*connection = None; diagnostics::emit(phase, failure, started.elapsed(), remaining);') in compact(function(io, 'query')))
    require('original unknown fence', compact(function(original, 'fence_unknown')) == compact(function(source, 'fence_unknown')))
    require('CAS diagnostic phase', compact(function(io, 'eval')).endswith(compact('deadline, Phase::Cas,)')))
    require('configuration helper', compact(function(source, 'configuration_error')) == compact('storage_io::emit(diagnostics, phase, Failure::ConfigurationMismatch, Duration::ZERO, Duration::ZERO,); StoreError::Configuration'))
    require('persistence helper', compact(function(source, 'persistence_error')) == compact('storage_io::emit(diagnostics, Phase::PersistenceHealth, Failure::UnhealthyPersistence, Duration::ZERO, Duration::ZERO,); StoreError::Unavailable'))
    health = function(source, 'check_health')
    require('exact health IO order', health.count('storage_io::info(') == 2 and health.count('storage_io::settings(') == 1 and health.index('storage_io::info(') < health.index('storage_io::settings(') < health.rindex('storage_io::info('))
    require('original server rejection', compact('if info_field(&info, "redis_version") != Some(REDIS_VERSION) { return Err(configuration_error(&mut self.diagnostics, Phase::ServerHealth,)); }') in compact(health))
    require('original query phases', compact('"server", deadline, Phase::ServerHealth,') in compact(health) and compact('"persistence", deadline, Phase::PersistenceHealth,') in compact(health) and compact('deadline, Phase::SettingsHealth,') in compact(health))
    settings = [('appendonly', 'yes'), ('appendfsync', 'always'), ('maxmemory-policy', 'noeviction'), ('no-appendfsync-on-rewrite', 'no'), ('aof-load-truncated', 'no')]
    positions = []
    for key, value in settings + [('maxmemory', None)]:
        expected = 'memory.as_str()' if value is None else '"' + value + '"'
        anchor = f'if !setting_matches(&settings, "{key}", {expected}) {{ return Err(configuration_error(&mut self.diagnostics, Phase::SettingsHealth,)); }}'
        normalized = compact(anchor)
        require('first configuration rejection ' + key, compact(health).count(normalized) == 1)
        positions.append(compact(health).index(normalized))
    require('configuration rejection order', positions == sorted(positions))
    positions = []
    for key, value in [('loading', '0'), ('aof_enabled', '1'), ('aof_last_write_status', 'ok'), ('aof_last_bgrewrite_status', 'ok')]:
        anchor = f'if info_field(&persistence, "{key}") != Some("{value}") {{ return Err(persistence_error(&mut self.diagnostics)); }}'
        normalized = compact(anchor)
        require('first persistence rejection ' + key, compact(health).count(normalized) == 1)
        positions.append(compact(health).index(normalized))
    require('persistence rejection order', positions == sorted(positions))
    require('exact diagnostic sites', health.count('configuration_error(') == 7 and health.count('persistence_error(') == 4 and 'diagnostics::emit(' not in health)
    require('no endpoint resource destructor', 'Drop for Diagnostics' not in io)
    return {'kind': 'finite source admission, not general equivalence or implementation refinement', 'checks': checked}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=ROOT)
    parser.add_argument('--candidate-root', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(check(args.root, args.candidate_root), indent=2))


if __name__ == '__main__':
    main()
