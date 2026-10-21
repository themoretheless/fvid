#!/usr/bin/env python3
"""Author zero-volume retriggers after an out-of-range sample offset."""
import hashlib
import json
import struct
from pathlib import Path
from generate_xm_key_off_regressions import generate as key_off, cell

OUT = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-offset-silence'

def generate():
    owned = key_off()
    base = owned['key-off.xm']
    pattern = 60 + struct.unpack_from('<I', base, 60)[0]
    body = base[pattern+9+struct.unpack_from('<H', base, pattern+7)[0]:]
    header = base[:pattern]
    def module(rows, sample_body):
        data = b''.join(rows)
        return header+struct.pack('<IBHH', 9, 0, len(rows), len(data))+data+sample_body
    files = {'video.mkv': owned['video.mkv']}
    cases = []
    for name, mute, unlooped, initial_silent in [
        ('effect-zero', cell(effect=0x0C), False, False),
        ('column-zero', cell(volume=0x10), False, False),
        ('natural-end-control', cell(effect=0x0C), True, False),
        ('no-gain-control', cell(effect=0x0C), False, True),
    ]:
        sample_body = bytearray(body)
        if unlooped:
            instrument_size = struct.unpack_from('<I', sample_body, 0)[0]
            sample_body[instrument_size+14] &= ~3
        rows = [cell(49,1,volume=0x10 if initial_silent else 0), cell(49,effect=9,parameter=255), mute, cell(49), cell()]
        control = rows.copy()
        control[3] = cell()
        files[name+'.xm'] = module(rows, sample_body)
        files[name+'-control.xm'] = module(control, sample_body)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',before_failure=not(unlooped or initial_silent),post_row=3,duration_ms=720))
    files['manifest.json'] = (json.dumps(dict(provenance='Author-created five-row XM scores. The existing owned signed16 waveform and authored VP9/PCM companion are reused. 9FF stops beyond the 64-frame sample; subsequent C00 or volume-column zero must keep a note-only retrigger silent. Independent controls omit only that zero-volume note. Natural sample-end and initially-zero gain are healthy controls. No private input,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():
        (OUT/name).write_bytes(data)
