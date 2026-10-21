#!/usr/bin/env python3
"""Compare missing valid XM slots with explicitly authored empty instruments."""
import hashlib
import json
import struct
from pathlib import Path
from generate_xm_key_off_regressions import generate as owned, cell

OUT = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-empty-slot'

def generate():
    source = owned()
    base = source['key-off.xm']
    pattern = 60+struct.unpack_from('<I',base,60)[0]
    header = base[:pattern]
    body = base[pattern+9+struct.unpack_from('<H',base,pattern+7)[0]:]
    empty = bytearray(243)
    struct.pack_into('<I',empty,0,243)
    empty[4:4+len(b'authored empty slot')] = b'authored empty slot'
    def module(rows, instruments=1):
        h = bytearray(header)
        struct.pack_into('<H',h,72,instruments)
        data = b''.join(rows)
        return bytes(h)+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body+bytes(empty)*(instruments-1)
    files = {'video.mkv':source['video.mkv']}
    cases = []
    for slot in [2,9,128]:
        for selection_only in [False,True]:
            name = f'slot-{slot}-'+('select-then-note' if selection_only else 'note')
            rows = [cell(49,1),cell(instrument=slot) if selection_only else cell(55,slot),cell(60),cell(49,1),cell(97)]
            files[name+'.xm'] = module(rows)
            files[name+'-empty-control.xm'] = module(rows,slot)
            cases.append(dict(name=name,file=name+'.xm',control=name+'-empty-control.xm',before_failure=True,post_row=2 if selection_only else 1,duration_ms=720,kind='missing-valid-slot'))
    for slot in [129,255]:
        name = f'out-of-format-{slot}-control'
        rows = [cell(49,1),cell(55,slot),cell(60),cell(49,1),cell(97)]
        control = rows.copy()
        control[1] = cell(55)
        files[name+'.xm'] = module(rows)
        files[name+'-empty-control.xm'] = module(control)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-empty-control.xm',before_failure=False,post_row=1,duration_ms=720,kind='ignored-out-of-format-slot'))
    name = 'initial-empty-slot-control'
    rows = [cell(49,2),cell(),cell(60),cell(49,1),cell(97)]
    files[name+'.xm'] = module(rows)
    files[name+'-empty-control.xm'] = module(rows,2)
    cases.append(dict(name=name,file=name+'.xm',control=name+'-empty-control.xm',before_failure=False,post_row=0,duration_ms=720,kind='initial-empty-slot'))
    name = 'populated-slot-control'
    rows = [cell(49,1),cell(55,1),cell(60),cell(49,1),cell(97)]
    files[name+'.xm'] = module(rows)
    files[name+'-empty-control.xm'] = module(rows,2)
    cases.append(dict(name=name,file=name+'.xm',control=name+'-empty-control.xm',before_failure=False,post_row=1,duration_ms=720,kind='populated-slot'))
    files['manifest.json'] = (json.dumps(dict(provenance='Ten author-created five-row XM scores. Valid but unpopulated slots2/9/128 are compared with the same rows and explicitly empty instruments. Instrument-only selection preserves active sample until the next note;explicit instrument1 recovers. Slots129/255 are ignored-format controls. The existing owned signed16 ramp and authored VP9/PCM companion are reused;no copied private media,codec parameters,encoder or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():
        (OUT/name).write_bytes(data)
