"""Owned XM EDx with literal tick-cell control scores; offline."""
import json,struct
from pathlib import Path
from generate_xm_volume_slide import generate as wrap
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-note-delay'
def C(n=0,i=0,v=0,e=0,p=0):return (n,i,v,e,p)
H=C()
def control(c,tick,events):return b''.join(cell(*event)for event in events)
def generate():
    cases=[
      ('delay-two',[(cell(49,1,effect=14,parameter=0xd2),[H,H,C(49,1),H])]),
      ('delay-one',[(cell(49,1,effect=14,parameter=0xd1),[H,C(49,1),H,H])]),
      ('delay-last-tick',[(cell(49,1,effect=14,parameter=0xd3),[H,H,H,C(49,1)])]),
      ('beyond-row',[(cell(49,1,effect=14,parameter=0xdf),[H]*4)]),
      ('not-carried',[(cell(49,1,effect=14,parameter=0xdf),[H]*4),(cell(),[H]*4)]),
      ('zero-new-note',[(cell(49,1,effect=14,parameter=0xd0),[C(49,1),H,H,H])]),
      ('zero-no-repeat',[(cell(49,1),[C(49,1),H,H,H]),(cell(effect=14,parameter=0xd0),[H]*4)]),
      ('old-note-continues',[(cell(49,1),[C(49,1),H,H,H]),(cell(53,effect=14,parameter=0xd2),[H,H,C(53),H])]),
      ('remembered-note',[(cell(49,1),[C(49,1),H,H,H]),(cell(effect=14,parameter=0xd2),[H,H,C(49),H])]),
      ('empty-history',[(cell(instrument=1,effect=14,parameter=0xd2),[H,H,C(i=1),H]),(cell(49),[C(49),H,H,H])]),
      ('volume-delayed',[(cell(49,1,0x30),[C(49,1,0x30),H,H,H]),(cell(53,volume=0x40,effect=14,parameter=0xd2),[H,H,C(53,v=0x40),H])]),
      ('pan-delayed',[(cell(49,1,effect=8,parameter=12),[C(49,1,e=8,p=12),H,H,H]),(cell(53,volume=0xc8,effect=14,parameter=0xd2),[H,H,C(53,v=0xc8),H])]),
      ('fine-column-down-ignored',[(cell(49,1,0x30),[C(49,1,0x30),H,H,H]),(cell(53,volume=0x85,effect=14,parameter=0xd2),[H,H,C(53),H])]),
      ('fine-column-up-ignored',[(cell(49,1,0x30),[C(49,1,0x30),H,H,H]),(cell(53,volume=0x95,effect=14,parameter=0xd2),[H,H,C(53),H])]),
      ('volume-column-before-trigger',[(cell(49,1,0x30),[C(49,1,0x30),H,H,H]),(cell(53,1,0x64,14,0xd2),[H,C(e=12,p=28),C(53,1),C(e=12,p=60)])]),
      ('pan-column-before-trigger',[(cell(49,1,effect=8,parameter=12),[C(49,1,e=8,p=12),H,H,H]),(cell(53,1,0xd4,14,0xd2),[H,C(e=8,p=8),C(53,1),C(e=8,p=124)])]),
      ('delayed-key-off',[(cell(49,1),[C(49,1),H,H,H]),(cell(97,effect=14,parameter=0xd2),[H,H,C(97),H])]),
      ('two-channels',[(cell(49,1,effect=14,parameter=0xd1)+cell(53,1,effect=14,parameter=0xd3),[(H,H),(C(49,1),H),(H,H),(H,C(53,1))])]),
      ('speed-one',[(cell(49,1,effect=15,parameter=1),[C(49,1),]),(cell(53,effect=14,parameter=0xd1),[H])]),
      ('fifteen-valid',[(cell(49,1,effect=15,parameter=16),[C(49,1)]+[H]*15),(cell(53,effect=14,parameter=0xdf),[H]*15+[C(53)])]),
      ('selected-instrument',[(cell(49,1),[C(49,1),H,H,H]),(cell(53,2,effect=14,parameter=0xd2),[H,H,C(53,2),H])]),
      ('beyond-selects-instrument',[(cell(49,1),[C(49,1),H,H,H]),(cell(53,2,effect=14,parameter=0xdf),[C(i=2),H,H,H]),(cell(53),[C(53),H,H,H])]),
    ]
    files=wrap(cases,control_tick=control)
    for name in ['selected-instrument','beyond-selects-instrument']:
        for suffix in ['', '-ticks']:
            key=name+suffix+'.xm';b=bytearray(files[key]);p=60+struct.unpack_from('<I',b,60)[0]
            at=p+9+struct.unpack_from('<H',b,p+7)[0];other=bytearray(b[at:])
            sample=struct.unpack_from('<I',other,0)[0];other[sample+16]=12
            struct.pack_into('<H',b,72,2);b+=other;files[key]=bytes(b)
    m=json.loads(files['manifest.json'])
    for c in m['cases']:c['events']=c.pop('volumes')
    m['provenance']='Owned signed16loop, literal tick-cell controls for EDx delay/ordering and instrument +12. No tools/network/private media.'
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
