"""Pin the already captured real encoder and the exact canonical reader parent."""
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
CANONICAL = HERE.with_name('source-save-load-canonical')
ENCODER = HERE.with_name('source-save-encoder')
sys.path.insert(0, str(CANONICAL))
from canonical_inputs import ROOT, NAMESPACE, retention, row_provider
sys.path.insert(0, str(ENCODER))
from encoder_inputs import MODES, stage

PARENT = '75fe473e92066b8ed0ae1f41d8ac21ec52ab1890'
ENCODER_SOURCE = 'dd82c2cdf2921bfdc04f11d032a2fcddd5771132'
ENCODER_NAMESPACE = 'SandproofSaveEncoderJoin'


def exact_package(package, commit):
    from hashlib import sha256
    rules = {file.name: retention.sha(file) for file in package.iterdir() if file.is_file()}
    for file, digest in rules.items():
        if sha256(retention.pinned(commit, package.relative_to(ROOT) / file)).hexdigest() != digest:
            raise ValueError('original package rule differs: ' + file)
    return rules
