"""Owned EEx held-cycle timing/effects against literal tick-cell controls."""
import json,struct
from pathlib import Path
from generate_xm_volume_slide import generate as wrap
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-pattern-delay'
def C(n=0,i=0,v=0,e=0,p=0):return(n,i,v,e,p)
H=C()
def pair(events):return [(event,H) for event in events]
def control(c,tick,events):return b''.join(cell(*event)for event in events)
def generate():
    cases=[
      ('hold-one',[(cell(49,1,effect=14,parameter=0xe1),[C(49,1)]+[H]*7)]),
      ('zero',[(cell(49,1,effect=14,parameter=0xe0),[C(49,1)]+[H]*3)]),
      ('maximum',[(cell(49,1,effect=14,parameter=0xef),[C(49,1)]+[H]*63)]),
      ('last-zero-wins',[(cell(49,1,effect=14,parameter=0xe2)+cell(53,1,effect=14,parameter=0xe0),[(C(49,1),C(53,1))]+[(H,H)]*3)]),
      ('last-two-wins',[(cell(49,1,effect=14,parameter=0xe0)+cell(53,1,effect=14,parameter=0xe2),[(C(49,1),C(53,1))]+[(H,H)]*11)]),
      ('column-keeps-sliding',[(cell(49,1,0x64,14,0xe2),[C(49,1)]+[C(e=12,p=v)for v in [60,56,52,48,44,40,36,32,28,24,20]])]),
      ('fine-column-once',[(cell(49,1,0x83,14,0xe2),[C(49,1,0x4d)]+[H]*11)]),
      ('A-at-held-zero',[(cell(49,1,effect=0xa,parameter=4)+cell(effect=14,parameter=0xe1),pair([C(49,1)]+[C(e=12,p=v)for v in [60,56,52,48,44,40,36]]))]),
      ('H-at-held-zero',[(cell(49,1,effect=0x11,parameter=2)+cell(effect=14,parameter=0xe1),pair([C(49,1,e=16,p=64)]+[C(e=16,p=v)for v in [62,60,58,56,54,52,50]]))]),
      ('P-at-held-zero',[(cell(49,1,effect=0x19,parameter=4)+cell(effect=14,parameter=0xe1),pair([C(49,1,e=8,p=128)]+[C(e=8,p=v)for v in [124,120,116,112,108,104,100]]))]),
      ('repeat-at-held-zero',[(cell(49,1,effect=14,parameter=0x92)+cell(effect=14,parameter=0xe1),pair([C(49,1),H,C(49),H,C(49),H,C(49),H]))]),
      ('delay-repeats',[(cell(49,1,effect=14,parameter=0xd2)+cell(effect=14,parameter=0xe1),pair([H,H,C(49,1),H,H,H,C(49,1),H]))]),
      ('ED0-at-held-zero',[(cell(49,1,effect=14,parameter=0xd0)+cell(effect=14,parameter=0xe1),pair([C(49,1),H,H,H,C(49,1),H,H,H]))]),
      ('E90-not-at-held-zero',[(cell(49,1,effect=14,parameter=0x90)+cell(effect=14,parameter=0xe1),pair([C(49,1)]+[H]*7))]),
      ('direct-volume-not-reset',[(cell(49,1,0x74,12,32)+cell(effect=14,parameter=0xe1),pair([C(49,1,e=12,p=32)]+[C(e=12,p=v)for v in [36,40,44,48,52,56,60]]))]),
      ('speed-one',[(cell(49,1,effect=15,parameter=1),[C(49,1)]),(cell(effect=14,parameter=0xe2),[H]*3)]),
      ('speed-one-A',[(cell(49,1)+cell(effect=15,parameter=1),[(C(49,1),H)]),(cell(effect=0xa,parameter=4)+cell(effect=14,parameter=0xe2),pair([H,C(e=12,p=60),C(e=12,p=56)]))]),
      ('jump-held',[(cell(49,1,effect=14,parameter=0xe1)+cell(effect=11,parameter=0),pair([C(49,1)]+[H]*7)),(cell(53,1)+cell(),pair([C(53,1)]+[H]*3))]),
      ('loop-held',[(cell(49,1,effect=14,parameter=0x60)+cell(),pair([C(49,1)]+[H]*3)),(cell(53,1,effect=14,parameter=0x61)+cell(effect=14,parameter=0xe1),pair([C(53,1)]+[H]*7+[C(53,1)]+[H]*7))]),
      ('jump-next-held',[(cell(49,1,effect=14,parameter=0xe1)+cell(effect=11,parameter=1),pair([C(49,1)]+[H]*7)),(cell(60,1)+cell(),[]),(cell(53,1)+cell(),pair([C(53,1)]+[H]*3))]),
    ]
    files=wrap(cases,control_tick=control)
    # Put row zero alone in pattern zero; the destination's unread row zero is skipped.
    key='jump-next-held.xm';b=bytearray(files[key]);at=60+struct.unpack_from('<I',b,60)[0]
    rows=struct.unpack_from('<H',b,at+5)[0];size=struct.unpack_from('<H',b,at+7)[0]
    payload=bytes(b[at+9:at+9+size]);body=bytes(b[at+9+size:]);stride=10
    struct.pack_into('<H',b,64,2);struct.pack_into('<H',b,70,2);b[80:336]=bytes([0,1])+bytes(254)
    files[key]=bytes(b[:at])+struct.pack('<IBHH',9,0,1,stride)+payload[:stride]+struct.pack('<IBHH',9,0,rows-1,size-stride)+payload[stride:]+body
    m=json.loads(files['manifest.json'])
    for c in m['cases']:c['events']=c.pop('volumes')
    m['provenance']='Owned EEx held cycles, literal tick-cell scores and flow controls. No tools/network/private media.'
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
