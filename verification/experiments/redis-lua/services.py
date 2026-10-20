"""Use the original complete physical-file validator for this module."""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.proof_services import check_files


FILES = tuple('SandboxModel/CenterStore/Lua/' + name + '.lean' for name in (
    'Syntax', 'RawSource', 'Calls', 'Flow', 'Paths', 'Scripts', 'Counterexamples', 'Audit'
)) + ('SandboxModel/CenterStore/Lua.lean',)


if __name__ == '__main__':
    check_files(FILES)
