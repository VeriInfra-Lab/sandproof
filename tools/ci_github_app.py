#!/usr/bin/env python3
"""Trusted host GitHub App client. Never log keys, JWTs or installation tokens."""
import argparse
from datetime import datetime
import json
import os
from pathlib import Path
import re
import stat
import time
import urllib.error
import urllib.request

REPOSITORY = 'ningZhang-cs/sandproof'
PERMISSIONS = {'actions': 'read', 'administration': 'write', 'metadata': 'read'}
CHECK_PERMISSIONS = {'actions': 'read', 'checks': 'write', 'metadata': 'read'}
SOURCE_PERMISSIONS = {'contents': 'read', 'metadata': 'read'}


class APIError(RuntimeError):
    def __init__(self, status, retry_after=0):
        self.status = status
        self.retry_after = retry_after
        super().__init__('GitHub API HTTP ' + str(status))


class Transport:
    def __init__(self, proxy):
        # Do not follow redirects with a management credential.
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, req, fp, code, msg, headers, newurl):
                return None
        self.opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({'https': proxy}), NoRedirect())

    def __call__(self, method, path, token, body=None):
        if not path.startswith('/') or '://' in path:
            raise ValueError('invalid API path')
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request('https://api.github.com' + path,
            data=data, method=method, headers={
                'Authorization': 'Bearer ' + token,
                'Accept': 'application/vnd.github+json',
                'Content-Type': 'application/json',
                'X-GitHub-Api-Version': '2022-11-28',
                'User-Agent': 'sandproof-ci-host'})
        try:
            with self.opener.open(request, timeout=25) as response:
                payload = response.read(8 * 1024 * 1024 + 1)
                if len(payload) > 8 * 1024 * 1024:
                    raise ValueError('API response exceeds host limit')
                return json.loads(payload) if payload else None
        except urllib.error.HTTPError as error:
            try:
                retry_after = min(900, max(0, int(error.headers.get('Retry-After', '0'))))
            except ValueError:
                retry_after = 60
            error.close()
            raise APIError(error.code, retry_after) from None
        except (urllib.error.URLError, TimeoutError, OSError):
            raise APIError('unavailable') from None


def private_key(path):
    details = path.lstat()
    if not stat.S_ISREG(details.st_mode) or details.st_uid != os.geteuid() or details.st_mode & 0o077:
        raise ValueError('App key must be an owner-only regular file')
    return path.read_bytes()


class GitHubApp:
    def __init__(self, client_id, key_path, transport, installation_id=None, clock=time.time,
                 permissions=None):
        self.permissions = dict(PERMISSIONS if permissions is None else permissions)
        if self.permissions not in (PERMISSIONS, CHECK_PERMISSIONS, SOURCE_PERMISSIONS):
            raise ValueError('unreviewed token permission profile')
        self.client_id = client_id
        self.key_path = key_path
        self.transport = transport
        self.installation_id = installation_id
        self.clock = clock
        self.token = None
        self.token_expires = 0
        self.repository_id = None

    def jwt(self):
        import jwt  # System PyJWT/cryptography, not a hand-written signing algorithm.
        now = int(self.clock())
        return jwt.encode({'iat': now - 60, 'exp': now + 540, 'iss': self.client_id},
                          private_key(self.key_path), algorithm='RS256')

    def verify_installation(self):
        jwt_token = self.jwt()
        app = self.transport('GET', '/app', jwt_token)
        self.app_id = app['id']
        installations = self.transport('GET', '/app/installations?per_page=100', jwt_token)
        if len(installations) >= 100:
            raise ValueError('installation inventory exceeds fixed host limit')
        matches = [item for item in installations
                   if item['account']['login'] == REPOSITORY.split('/')[0]]
        if len(matches) != 1:
            raise ValueError('install this App on the repository owner first')
        installation = matches[0]
        if self.installation_id is not None and installation['id'] != self.installation_id:
            raise ValueError('installation identity changed')
        if installation.get('suspended_at') or installation['repository_selection'] != 'selected':
            raise ValueError('App must be active and restricted to selected repositories')
        allowed = [dict(PERMISSIONS, **extra) for extra in
                   ({}, {'checks':'write'}, {'contents':'read'}, {'checks':'write', 'contents':'read'})]
        granted = installation['permissions']
        if (granted not in allowed
                or any(granted.get(key) != value for key, value in self.permissions.items())):
            raise ValueError('App permissions differ from the reviewed CI profiles')
        self.installation_id = installation['id']
        # Read the installation inventory before accepting it for unattended work.
        result = self.transport('POST', '/app/installations/' + str(self.installation_id) + '/access_tokens',
                                jwt_token, {'permissions': self.permissions})
        repositories = self.transport('GET', '/installation/repositories?per_page=100', result['token'])
        if repositories['total_count'] != 1 or repositories['repositories'][0]['full_name'] != REPOSITORY:
            raise ValueError('App installation must select only ' + REPOSITORY)
        self.repository_id = repositories['repositories'][0]['id']
        self.token = None
        self.token_expires = 0
        return {'app_id': app['id'], 'client_id': self.client_id,
                'installation_id': self.installation_id, 'repository': REPOSITORY,
                'repository_id': self.repository_id, 'permissions': installation['permissions']}

    def access_token(self):
        if self.token and self.clock() < self.token_expires - 120:
            return self.token
        if self.repository_id is None:
            raise ValueError('installation scope has not been verified')
        result = self.transport('POST', '/app/installations/' + str(self.installation_id) + '/access_tokens',
            self.jwt(), {'repository_ids': [self.repository_id], 'permissions': self.permissions})
        if result['permissions'] != self.permissions:
            raise ValueError('installation token permissions changed')
        self.token = result['token']
        self.token_expires = datetime.fromisoformat(result['expires_at'].replace('Z', '+00:00')).timestamp()
        return self.token

    def request(self, method, path, body=None):
        # A failed mutation may have succeeded at GitHub. Caller reconciles;
        # this client NEVER blindly retries POST/DELETE after transport failure.
        prefix = '/repos/' + REPOSITORY + '/'
        suffix = path[len(prefix):] if path.startswith(prefix) else ''
        ordinary = suffix.startswith('actions/') and self.permissions == PERMISSIONS
        checks = self.permissions == CHECK_PERMISSIONS and (
            (method == 'GET' and re.fullmatch(r'commits/[0-9a-f]{40}/check-runs\?per_page=100&filter=all', suffix))
            or (method == 'POST' and suffix == 'check-runs')
            or (method == 'PATCH' and re.fullmatch(r'check-runs/[0-9]+', suffix)))
        if not ordinary and not checks:
            raise ValueError('repository API outside CI scope')
        try:
            return self.transport(method, path, self.access_token(), body)
        except APIError as error:
            if error.status == 401:
                self.token = None
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client-id', required=True)
    parser.add_argument('--key', type=Path, required=True)
    parser.add_argument('--proxy', default='http://127.0.0.1:7897')
    args = parser.parse_args()
    try:
        app = GitHubApp(args.client_id, args.key, Transport(args.proxy))
        print(json.dumps(app.verify_installation()))
    except (APIError, ValueError, OSError):
        print('App scope verification failed; check installation and permissions (no credential printed).')
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
