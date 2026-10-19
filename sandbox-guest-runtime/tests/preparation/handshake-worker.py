#!/usr/bin/python3
"""Private handshake barrier fixture; deliberately never sends a valid HELLO."""
from pathlib import Path
import socket
import sys

gate_path = Path(sys.argv[sys.argv.index('--gate') + 1])
gate = socket.socket(socket.AF_UNIX)
gate.connect(str(gate_path))
notify = socket.socket(socket.AF_UNIX)
# <fixture>/gates/<one preparation>/launch
notify.connect(str(gate_path.parents[2] / 'notify'))
notify.sendall(b'R')
if notify.recv(1) == b'B':
    gate.sendall(b'BAD!')
notify.recv(1)
