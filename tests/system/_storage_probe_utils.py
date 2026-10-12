"""Pure storage-probe utilities; no service launcher or filesystem provisioning."""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import hashlib
from pathlib import Path
import subprocess


REPO = Path(__file__).resolve().parents[2]


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def git(*args):
    # A root-only probe must not refresh the user's index under its private umask.
    return command("git", "--no-optional-locks", "-c", f"safe.directory={REPO}",
                   "-C", str(REPO), *args)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def trusted(path, private=False):
    assert path.is_absolute() and path.resolve(strict=True) == path
    for part in (path, *path.parents):
        metadata = part.stat()
        assert metadata.st_uid == 0 and metadata.st_mode & 0o022 == 0
    if private:
        assert path.stat().st_mode & 0o077 == 0
