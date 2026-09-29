"""Fixed management samples shared by synthetic SDK and real KVM probes.

Expected query matches refer to two explicitly created fixture records. This
module does not filter records, emulate the controller, or advance VM state.
"""

from datetime import timedelta

from e2b import SandboxQuery, SandboxState
from e2b.exceptions import SandboxException


DEFAULT_LIFETIME = 300
REJECTED_LIFETIMES = ('zero', 'negative', 'above_maximum')
SPECIAL_METADATA = {'reserved &+%=': '雪 =+&/%'}
CONTEXT_KEYS = ['SP_CTX_COMMON', 'SP_CTX_EMPTY', 'SP_CTX_UTF8', 'SP_CTX_ONLY_A',
                'SP_CTX_ONLY_B', 'SP_CTX_TRANSIENT', 'SP_META_ONLY']
LIST_DETAIL_FIELDS = ('sandbox_id', 'template_id', 'metadata', 'started_at', 'end_at',
                      'state', 'cpu_count', 'memory_mb', 'envd_version')


def creation_contexts(run):
    # Preserve empty/Unicode/shell text; metadata must never become guest env.
    return [
        ({'SP_CTX_COMMON': label, 'SP_CTX_EMPTY': '',
          'SP_CTX_UTF8': f'{label} 雪 "quotes" $HOME; =+&%\nsecond line',
          'SP_CTX_ONLY_' + label: run},
         {'context_run': run, 'label': label, 'empty': '',
          'reserved &=+%': f'{label} 雪 =+&/%', 'SP_META_ONLY': 'metadata only'})
        for label in ('A', 'B')
    ]


def validate_context(actual, expected):
    assert actual == {key: expected.get(key) for key in CONTEXT_KEYS}, (actual, expected)


def lifetime_values(maximum):
    return [('omitted', None), ('zero', 0), ('negative', -1),
            ('below_maximum', maximum - 1), ('at_maximum', maximum),
            ('above_maximum', maximum + 1)]


def lifetime_arguments(case, value):
    return {} if case == 'omitted' else {'timeout': value}


def validate_bad_lifetime(error):
    # Management HTTP 400 is distinct from envd RPC InvalidArgumentException.
    assert type(error) is SandboxException and error.status_code == 400


def validate_created_lifetime(info, case, value, default):
    expected = default if case == 'omitted' else value
    assert (info.end_at - info.started_at).total_seconds() == expected


def validate_identity(sandbox, expected):
    assert sandbox.sandbox_id == expected


def validate_kill(result, expected):
    assert result is expected


def validate_list(items, expected):
    actual = [item.sandbox_id for item in items]
    assert actual == expected, (actual, expected)


def validate_list_detail(item, detail):
    for field in LIST_DETAIL_FIELDS:
        assert getattr(item, field) == getattr(detail, field), field


def query_cases(common, first, second, latest):
    """The existing two-owner-record query matrix, with fresh SDK queries."""
    newest = [second, first]
    return [
        ('default_newest_first', newest, {}),
        ('descending', newest, {'query': SandboxQuery(metadata=common), 'order': 'desc'}),
        ('ascending', [first, second], {'query': SandboxQuery(metadata=common), 'order': 'asc'}),
        ('metadata_unicode_reserved', newest, {'query': SandboxQuery(metadata=SPECIAL_METADATA)}),
        ('metadata_and', [first], {'query': SandboxQuery(metadata={**common, 'colour': 'red'})}),
        ('metadata_no_match', [], {'query': SandboxQuery(metadata={**common, 'colour': 'absent'})}),
        ('running', newest, {'query': SandboxQuery(state=[SandboxState.RUNNING])}),
        ('both_states', newest, {'query': SandboxQuery(state=[SandboxState.PAUSED, SandboxState.RUNNING])}),
        ('paused', [], {'query': SandboxQuery(state=[SandboxState.PAUSED])}),
        ('template_base', newest, {'query': SandboxQuery(template='base')}),
        ('template_absent', [], {'query': SandboxQuery(template='unknown/base')}),
        ('inclusive_started_after', [second], {'query': SandboxQuery(started_after=latest)}),
        ('after_latest', [], {'query': SandboxQuery(started_after=latest + timedelta(microseconds=1))}),
        ('combined', [second], {'query': SandboxQuery(metadata={**common, 'colour': 'blue'},
            state=[SandboxState.RUNNING], template='base', started_after=latest), 'order': 'asc'}),
        ('maximum_page_size', newest, {'limit': 100}),
    ]
