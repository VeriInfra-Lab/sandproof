"""Pin both checked proof providers, including the retained actual encoder variant."""
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
SERDE = HERE.with_name('source-serde-ports')
ROUNDTRIP = HERE.with_name('source-save-load-roundtrip')
sys.path.insert(0, str(ROUNDTRIP))
from roundtrip_inputs import (ROOT, retention, row_provider, exact_package, ENCODER,
                              ENCODER_SOURCE, ENCODER_NAMESPACE, MODES)
sys.path.insert(0, str(SERDE))

PARENT = 'a1275a95df622bc4846fe01769acc21a28565553'
ROUNDTRIP_COMMIT = '2edc1a6c14c0b379dd7dc4977631adde488c97ca'
