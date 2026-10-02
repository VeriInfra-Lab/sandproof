"""Fail closed when Python would remove system acceptance checks/actions.

Load this sibling by path so direct CLIs and importlib-based component tests
use the same guard without changing sys.path. Offline guest probes retain an
equivalent inline check because they are uploaded as standalone files.
"""

import sys


def require_assertions(error_type=RuntimeError):
    """Reject -O, -OO and PYTHONOPTIMIZE before SDK imports or resource work."""
    if sys.flags.optimize:
        raise error_type("optimized_python")


if __name__ == "__main__":
    require_assertions()
