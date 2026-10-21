#!/usr/bin/env python3
"""Author no-envelope release state against independent volume-zero controls."""
import hashlib
import json
import struct
from pathlib import Path
from generate_xm_key_off_regressions import generate as immediate, cell

OUT = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-key-off-volume-state'

def generate():
    authored = immediate()
    base = authored['key-off.xm']
    at = 60+struct.unpack_from('<I',base,60)[0]
    body = base[at+9+struct.unpack_from('<H',base,at+7)[0]:]
    header = base[:at]
    def module(rows, sample_body=body):
        data = b''.join(rows)
        return header+struct.pack('<IBHH',9,0,len(rows),len(data))+data+sample_body
    files = {'video.mkv':authored['video.mkv']}
    cases = []
    def add(name, release, post, bad, kind):
        actual = [cell(49,1),release,post,cell(97)]
        control = [cell(49,1),cell(effect=0x0C),post,cell(effect=0x0C)]
        files[name+'.xm'] = module(actual)
        files[name+'-control.xm'] = module(control)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',before_failure=bad,kind=kind,duration_ms=600,post_row=2))
    add('note-off-note-only',cell(97),cell(49),True,'unexpected-note-resume')
    add('k00-note-only',cell(effect=0x14),cell(49),True,'unexpected-note-resume')
    add('note-off-volume-only',cell(97),cell(effect=0x0C,parameter=32),True,'blocked-volume-restore')
    add('k00-volume-only',cell(effect=0x14),cell(effect=0x0C,parameter=32),True,'blocked-volume-restore')
    add('note-off-instrument-only',cell(97),cell(instrument=1),True,'blocked-volume-restore')
    add('note-off-fine-volume',cell(97),cell(volume=0x93),True,'blocked-volume-restore')
    add('explicit-instrument-control',cell(97),cell(49,1),False,'healthy-control')
    add('no-restore-control',cell(97),cell(),False,'healthy-control')
    add('explicit-volume-note-control',cell(97),cell(49,volume=0x50),False,'healthy-control')
    add('global-volume-control',cell(97),cell(effect=0x10,parameter=64),False,'healthy-control')
    name = 'k03-note-only'
    files[name+'.xm'] = module([cell(49,1,effect=0x14,parameter=3),cell(49),cell(97)])
    files[name+'-control.xm'] = module([cell(49,1,effect=0x0F,parameter=3),cell(volume=0x10,effect=0x0F,parameter=3),cell(49,effect=0x0F,parameter=6),cell(effect=0x0C)])
    cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',before_failure=True,kind='unexpected-note-resume',duration_ms=480,post_row=1))
    for name,actual,control in [
        ('note-off-same-row-volume', [cell(49,1),cell(97,volume=0x30),cell(),cell(97)], [cell(49,1),cell(volume=0x30),cell(),cell(effect=0x0C)]),
        ('k00-same-row-instrument', [cell(49,1),cell(50,1,effect=0x14),cell(),cell(97)], [cell(49,1),cell(instrument=1),cell(),cell(effect=0x0C)]),
    ]:
        files[name+'.xm'] = module(actual)
        files[name+'-control.xm'] = module(control)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',before_failure=True,kind='blocked-volume-restore',duration_ms=600,post_row=1))
    unlooped = bytearray(body)
    instrument_size = struct.unpack_from('<I',unlooped,0)[0]
    unlooped[instrument_size+14] &= ~3
    name = 'natural-end-control'
    files[name+'.xm'] = module([cell(49,1),cell(97),cell(effect=0x0C,parameter=32),cell(97)],bytes(unlooped))
    files[name+'-control.xm'] = module([cell(49,1),cell(effect=0x0C),cell(effect=0x0C,parameter=32),cell(effect=0x0C)],bytes(unlooped))
    cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',before_failure=False,kind='healthy-control',duration_ms=600,post_row=2))
    files['manifest.json'] = (json.dumps({'provenance':'Authored no-envelope XM release/restore rows versus independent C00 or split-row volume-zero controls. Signed16 looping wave and authored VP9/PCM companion inherited from the owned key-off generator. No private media,network,encoder or copied codec parameters. Controls test channel-volume state,not full FT2 bit-exact synthesis.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():
        (OUT/name).write_bytes(data)
