"""Resolve one protected test promotion from a real GitHub merge group."""
import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import subprocess

from tools.ci_github_app import REPOSITORY, Transport

REPOSITORY_ID = 1359710455
TARGET = 'refs/heads/test'
QUEUE_PREFIX = 'refs/heads/gh-readonly-queue/test/'
QUERY = '''
query ($ref: String!) {
  repository(owner: "VeriInfra-Lab", name: "sandproof") {
    nameWithOwner databaseId
    base: ref(qualifiedName: "refs/heads/test") { target { oid } }
    integration: ref(qualifiedName: "refs/heads/integration") { target { oid } }
    main: ref(qualifiedName: "refs/heads/main") { target { oid } }
    group: ref(qualifiedName: $ref) { target { oid } }
    mergeQueue(branch: "test") {
      entries(first: 2) {
        totalCount pageInfo { hasNextPage }
        nodes {
          id baseCommit { oid }
          pullRequest {
            number title state isDraft isCrossRepository
            headRefName headRefOid baseRefName baseRefOid
            headRepository { nameWithOwner databaseId }
            baseRepository { nameWithOwner databaseId }
          }
        }
      }
    }
  }
}
'''


def object_id(value):
    if not isinstance(value, str) or re.fullmatch(r'[0-9a-f]{40}', value) is None:
        raise ValueError('invalid Git object identity')
    return value


@dataclass(frozen=True)
class QueueGroup:
    ref: str
    sha: str
    base: str

    def __post_init__(self):
        if not isinstance(self.ref, str) or re.fullmatch(
                re.escape(QUEUE_PREFIX) + r'[A-Za-z0-9][A-Za-z0-9._/-]{0,180}', self.ref) is None:
            raise ValueError('expected an exact test queue ref')
        if '..' in self.ref or self.ref.endswith(('/', '.')) or '//' in self.ref:
            raise ValueError('invalid queue ref')
        object_id(self.sha)
        object_id(self.base)
        if self.sha == self.base:
            raise ValueError('queue group must be a separate commit')

    @classmethod
    def from_event(cls, event):
        group = event['merge_group']
        if (event.get('action') != 'checks_requested' or group.get('base_ref') != TARGET
                or event.get('repository', {}).get('id') != REPOSITORY_ID):
            raise ValueError('expected a real test merge_group request')
        return cls(group['head_ref'], group['head_sha'], group['base_sha'])

    def observe(self, graphql, commit):
        # Require the server to identify the singleton queue entry. PR numbers
        # embedded in queue branch names are not an authority.
        body = graphql(QUERY, {'ref': self.ref})
        if body.get('errors'):
            raise ValueError('GitHub merge queue metadata unavailable')
        repo = body['data']['repository']
        if (repo.get('databaseId') != REPOSITORY_ID or repo.get('nameWithOwner') != REPOSITORY
                or (repo.get('group') or {}).get('target', {}).get('oid') != self.sha
                or (repo.get('base') or {}).get('target', {}).get('oid') != self.base):
            raise ValueError('current queue ref or test base differs from the requested group')
        queue = repo.get('mergeQueue')
        entries = queue.get('entries') if queue else None
        if (not entries or entries.get('totalCount') != 1
                or entries.get('pageInfo', {}).get('hasNextPage') is not False
                or len(entries.get('nodes', ())) != 1):
            raise ValueError('requires exactly one live test queue member')
        entry, = entries['nodes']
        pr = entry['pullRequest']
        source, head = pr['headRefName'], object_id(pr['headRefOid'])
        if (not entry.get('id')
                or (entry.get('baseCommit') or {}).get('oid') != self.base
                or pr.get('state') != 'OPEN' or pr.get('isDraft') is not False
                or pr.get('isCrossRepository') is not False
                or pr.get('baseRefName') != 'test' or pr.get('baseRefOid') != self.base
                or source not in ('integration', 'main') or head == self.base
                or (repo.get(source) or {}).get('target', {}).get('oid') != head
                or type(pr.get('number')) is not int or pr['number'] <= 0
                or not isinstance(pr.get('title'), str)
                or any((pr.get(side) or {}).get('databaseId') != REPOSITORY_ID
                       or (pr.get(side) or {}).get('nameWithOwner') != REPOSITORY
                       for side in ('headRepository', 'baseRepository'))):
            raise ValueError('queue member is not the exact protected promotion PR')
        actual = commit(self.sha)
        if (actual.get('sha') != self.sha
                or [parent['sha'] for parent in actual.get('parents', ())] != [self.base, head]):
            raise ValueError('group is not a singleton merge of the observed base and PR head')
        return dict(entry=entry['id'], pull=pr['number'], source=source, head=head,
                    base=self.base, tree=object_id(actual['tree']['sha']), title=pr['title'])


def graphql_lookup(transport, token):
    return lambda query, variables: transport('POST', '/graphql', token,
                                               {'query': query, 'variables': variables})


def checked_out_commit(sha):
    # cat-file preserves parent OIDs even when actions/checkout used depth 1.
    actual = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    if actual != sha:
        raise ValueError('Actions checked out a different queue commit')
    headers = subprocess.check_output(['git', 'cat-file', '-p', sha], text=True).split('\n\n', 1)[0]
    parents = [{'sha': line[7:]} for line in headers.splitlines() if line.startswith('parent ')]
    tree, = [line[5:] for line in headers.splitlines() if line.startswith('tree ')]
    return dict(sha=actual, parents=parents, tree={'sha': tree})


def pr_event(row):
    return {'pull_request': dict(number=row['pull'], title=row['title'],
        head=dict(ref=row['source'], sha=row['head'], repo={'id': REPOSITORY_ID}),
        base=dict(ref='test', sha=row['base'], repo={'id': REPOSITORY_ID}))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--event', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    if os.environ.get('GITHUB_EVENT_NAME') != 'merge_group':
        raise ValueError('queue metadata can only be sourced from merge_group')
    group = QueueGroup.from_event(json.loads(args.event.read_text()))
    if (os.environ.get('GITHUB_REPOSITORY') != REPOSITORY
            or os.environ.get('GITHUB_SHA') != group.sha or os.environ.get('GITHUB_REF') != group.ref):
        raise ValueError('Actions queue environment disagrees with the requested object')
    lookup = graphql_lookup(Transport(os.environ.get('https_proxy') or os.environ.get('HTTPS_PROXY')),
                            os.environ['GH_TOKEN'])
    meta = group.observe(lookup, checked_out_commit)
    with args.output.open('x') as stream:
        json.dump(pr_event(meta), stream)


if __name__ == '__main__':
    main()
