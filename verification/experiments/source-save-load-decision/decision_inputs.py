"""Pin the exact prior full key decoder and current roundtrip parent."""
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
ROUNDTRIP = HERE.with_name('source-save-load-roundtrip')
COLLECTOR = HERE.with_name('source-save-load-collector')
sys.path.insert(0, str(ROUNDTRIP))
from roundtrip_inputs import exact_package
sys.path.insert(0, str(COLLECTOR))
from collector_inputs import ROOT, retention, row_provider, SOURCE_FILES, NAMESPACE, MODES, stage

PARENT = '2edc1a6c14c0b379dd7dc4977631adde488c97ca'
COLLECTOR_SOURCE = 'cd6f6b3c8365e1dd05fe350443f0ff848e33c557'
