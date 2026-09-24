"""Thin accessors to the local E2B harness (api, Redis, Postgres).

Kept separate so the observer/collector do not embed shell details.
"""
import json
import os
import subprocess
import urllib.error
import urllib.request

API = os.environ.get("API", "http://localhost:3000")
API_KEY = os.environ.get("API_KEY")
TEAM = os.environ.get("TEAM", "834777bd-9956-45ca-b088-9bac9290e2ac")
TPL = os.environ.get("TPL", "11111111-1111-1111-1111-111111111111")
PG_DB = os.environ.get("PG_DB", "mydatabase")

RUNNING_PREFIX = f"sandbox:storage:{{{TEAM}}}:sandboxes:"
INDEX_KEY = f"sandbox:storage:{{{TEAM}}}:index"


def _docker(args):
    return subprocess.run(args, capture_output=True, text=True, timeout=30).stdout


def redis_get(key):
    out = _docker(["docker", "exec", "redis", "redis-cli", "GET", key])
    value = out.strip()
    return value or None


def redis_keys(pattern):
    out = _docker(["docker", "exec", "redis", "redis-cli", "KEYS", pattern])
    return [line for line in out.splitlines() if line]


def redis_smembers(key):
    out = _docker(["docker", "exec", "redis", "redis-cli", "SMEMBERS", key])
    return [line for line in out.splitlines() if line]


def psql(sql):
    return _docker(["docker", "exec", "postgres", "psql", "-U", "postgres",
                    "-d", PG_DB, "-tAc", sql]).strip()


DUMMY_INTROSPECT = os.environ.get("DUMMY_INTROSPECT", "http://localhost:5018")


def node_sandboxes():
    """The ids held by the (dummy) node's in-memory store, via its read-only
    introspection endpoint."""
    try:
        with urllib.request.urlopen(DUMMY_INTROSPECT + "/sandboxes", timeout=5) as resp:
            return json.loads(resp.read().decode()).get("sandboxes", [])
    except Exception:
        return []


def http(method, path, body=None, timeout=120):
    if not API_KEY:
        raise RuntimeError("Set API_KEY to the local E2B test key")
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(API + path, data=data, method=method)
    req.add_header("X-API-Key", API_KEY)
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()
    except Exception as e:  # timeout / connection
        return None, str(e)
