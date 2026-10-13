"""Native AV1 configuration fields for explicit offline fixture generation."""
import atexit
import functools
import json
import subprocess
from pathlib import Path

_ROOT = Path(__file__).resolve().parents[1]
_PROCESS = None

def _close():
    if _PROCESS is not None:
        _PROCESS.stdin.close()
        _PROCESS.wait()

atexit.register(_close)

@functools.lru_cache(maxsize=None)
def configuration(sequence):
    global _PROCESS
    if not sequence or sequence[0] >> 3 & 15 != 1 or not sequence[0] & 2:
        raise ValueError('expected sized sequence OBU')
    at = 1 + bool(sequence[0] & 4)
    length = shift = 0
    while True:
        byte = sequence[at]
        at += 1
        length |= (byte & 127) << shift
        shift += 7
        if not byte & 128:
            break
        if shift >= 56:
            raise ValueError('oversized sequence OBU length')
    if at + length != len(sequence):
        raise ValueError('expected exactly one sequence OBU')
    if _PROCESS is None:
        build = subprocess.run(['cargo', 'build', '--locked', '--offline',
            '--no-default-features', '--example', 'av1_configuration_fields',
            '--message-format=json'], cwd=_ROOT, check=True, capture_output=True, text=True)
        executable = next(json.loads(line)['executable'] for line in build.stdout.splitlines()
            if json.loads(line).get('executable') and
            json.loads(line).get('target', {}).get('name') == 'av1_configuration_fields')
        _PROCESS = subprocess.Popen([executable], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, text=True)
    _PROCESS.stdin.write(json.dumps(list(sequence[at:])) + '\n')
    _PROCESS.stdin.flush()
    response = json.loads(_PROCESS.stdout.readline())
    if 'error' in response:
        raise ValueError(response['error'])
    return bytes([0x81, *response['fields'], 0]) + sequence
