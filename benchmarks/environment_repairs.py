"""Reviewed, explicit environment variants; never original-environment results."""
import hashlib
import io
import json
from pathlib import Path
import tarfile

from tools.benchmark_tasks import InvalidInput

PRUNED_SOURCE = 'oci-pip-cache-pruned'
SOURCE_KINDS = ('oci', 'dockerfile', PRUNED_SOURCE)
REPAIRS = {
    'mteb-leaderboard': {
        'files_sha256': '8cada508d2131a149c15846c56e390f693ebae74078f4c77a1257812b47e2c9c',
        'image': 'alexgshaw/mteb-leaderboard:20251031',
        'digest': 'sha256:968de9456bfc53728f7df0cb6c9ce33e17bd0e5563ddfe031e6ed2964965aecf'},
    'mteb-retrieve': {
        'files_sha256': 'affc7ca35a9e252e75873583ba7334fe83c9197c48eaa6e68d912f863e6e8d53',
        'image': 'alexgshaw/mteb-retrieve:20251031',
        'digest': 'sha256:99dd676a21fd3124b7b8164cd93e8f0c51f1fdab2f84a831c61049eff78d063d'},
}


def repair_profile(entry):
    profile = REPAIRS.get(entry['name'])
    if (profile is None or entry.get('files_sha256') != profile['files_sha256']
            or entry.get('upstream_image_reference') != profile['image']):
        raise InvalidInput('environment repair requires the registered task and image')
    return profile


def validate_source_kind(source_kind, entries):
    if source_kind not in SOURCE_KINDS:
        raise InvalidInput('unknown environment source kind')
    if source_kind == PRUNED_SOURCE:
        for entry in entries:
            repair_profile(entry)


def archive_repaired_context(entry, output):
    profile = repair_profile(entry)
    script = Path(__file__).with_name('guest_pip_cache.py').read_bytes()
    base_image = 'docker.io/' + profile['image'].split(':', 1)[0] + '@' + profile['digest']
    command = ['/usr/local/bin/python3', '-I', '-S', '-c', script.decode()]
    dockerfile = ('FROM ' + base_image + '\nRUN ' + json.dumps(command) + '\n').encode()
    with tarfile.open(output, 'x', format=tarfile.PAX_FORMAT) as archive:
        member = tarfile.TarInfo('Dockerfile')
        member.size, member.mode, member.mtime = len(dockerfile), 0o644, 0
        archive.addfile(member, io.BytesIO(dockerfile))
    return {'kind': PRUNED_SOURCE, 'original_environment': False,
            'original_image_reference': profile['image'], 'base_image': base_image,
            'base_image_digest': profile['digest'], 'task_files_sha256': entry['files_sha256'],
            'removed_path': '/root/.cache/pip',
            'helper_sha256': hashlib.sha256(script).hexdigest(),
            'dockerfile_sha256': hashlib.sha256(dockerfile).hexdigest(),
            'context_sha256': hashlib.sha256(Path(output).read_bytes()).hexdigest()}
