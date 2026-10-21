"""Owned XM Rxy against independent literal tick cells; offline."""
import json
from pathlib import Path
from generate_xm_volume_slide import generate as wrap
from generate_xm_note_delay import C,H,control
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-multi-retrigger'
def pair(events):return [(event,H)for event in events]
def generate():
    levels=[[32]*4,[31,30,29,28],[30,28,26,24],[28,24,20,16],[24,16,8,0],[16,0,0,0],[22,14,8,5],[16,8,4,2],[32]*4,[33,34,35,36],[34,36,38,40],[36,40,44,48],[40,48,56,64],[48,64,64,64],[48,64,64,64],[64]*4]
    cases=[]
    for op,values in enumerate(levels):
        cases.append((f'volume-op-{op:x}',[(cell(49,1,0x30),[C(49,1,0x30),H,H,H]),(cell(effect=27,parameter=op*16+1),[C(49,v=v+16)for v in values])]))
    cases += [
      ('period-carries',[(cell(49,1,effect=27,parameter=0x83),[C(49,1),H,C(49),H]),(cell(effect=27),[H,C(49),H,H])]),
      ('volume-memory',[(cell(49,1,0x30,27,0x93),[C(49,1,0x30),H,H,C(49,v=0x30)]),(cell(effect=27),[H,H,C(49,v=0x31),H])]),
      ('direct-volume-gate',[(cell(49,1,0x30,27,0x83),[C(49,1,0x30),H,H,C(49,v=0x30)])]),
      ('direct-zero-gate',[(cell(49,1,0x10,27,0x93),[C(49,1,0x10),H,C(49,v=0x10),H]),(cell(effect=27),[H,C(49,v=0x11),H,H])]),
      ('fine-zero-gate',[(cell(49,1,0x13),[C(49,1,0x13),H,H,H]),(cell(volume=0x85,effect=27,parameter=0x93),[C(e=12,p=0),H,C(49,v=0x11),H])]),
      ('pan-zero-gate',[(cell(49,1,0xc0,27,0x83),[C(49,1,0xc0),H,C(49,v=0xc0),H])]),
      ('pan-nonzero-gate',[(cell(49,1,0xc8,27,0x83),[C(49,1,0xc8),H,H,C(49,v=0xc8)])]),
      ('instrument-resets-counter',[(cell(49,1,effect=27,parameter=0x83),[C(49,1),H,C(49),H]),(cell(instrument=1,effect=27,parameter=0x83),[C(i=1),H,C(49),H])]),
      ('note-without-instrument-keeps-counter',[(cell(49,1,effect=27,parameter=0x83),[C(49,1),H,C(49),H]),(cell(53,effect=27,parameter=0x83),[C(53),C(53),H,H])]),
      ('E9-resets-counter',[(cell(49,1,effect=27,parameter=0x83),[C(49,1),H,C(49),H]),(cell(effect=14,parameter=0x92),[H,H,C(49),H]),(cell(effect=27,parameter=0x83),[H,H,C(49),H])]),
      ('ED-resets-counter',[(cell(49,1,effect=27,parameter=0x83),[C(49,1),H,C(49),H]),(cell(effect=14,parameter=0xd2),[H,H,C(49),H]),(cell(effect=27,parameter=0x83),[H,H,C(49),H])]),
      ('E90-keeps-counter',[(cell(49,1,effect=27,parameter=0x83),[C(49,1),H,C(49),H]),(cell(effect=14,parameter=0x90),[C(49),H,H,H]),(cell(effect=27,parameter=0x83),[H,C(49),H,H])]),
      ('held-cycle',[(cell(49,1,effect=27,parameter=0x83)+cell(effect=14,parameter=0xe1),pair([C(49,1),H,C(49),H,H,C(49),H,H]))]),
      ('held-cycle-column-gate',[(cell(49,1,0x30,27,0x83)+cell(effect=14,parameter=0xe1),pair([C(49,1,0x30),H,H,C(49,v=0x30),H,H,C(49,v=0x30),H]))]),
      ('speed-one-counter',[(cell(49,1)+cell(effect=15,parameter=1),[(C(49,1),H)]),(cell(effect=27,parameter=0x83)+cell(effect=14,parameter=0xe2),pair([H,H,C(49)]))]),
      ('empty-history',[(cell(effect=27,parameter=0x83),[H]*4),(cell(49,1,effect=27,parameter=0x83),[C(49,1),H,C(49),H])]),
      ('two-channel-memory',[(cell(49,1,effect=27,parameter=0x82)+cell(53,1,effect=27,parameter=0x83),[(C(49,1),C(53,1)),(C(49),H),(H,C(53)),(C(49),H)]),(cell(effect=27)+cell(effect=27),[(H,H),(C(49),C(53)),(H,H),(C(49),H)])]),
      ('reset-empty-arguments',[(cell(49,1,effect=27),[C(49,1),C(49),C(49),C(49)]),(cell(effect=27,parameter=0x93),[H,H,C(49),H])]),
    ]
    files=wrap(cases,control_tick=control)
    m=json.loads(files['manifest.json'])
    for c in m['cases']:c['events']=c.pop('volumes')
    m['provenance']='Owned Rxy volume table, literal tick events, column gates, persistent counters and E9/ED/EE interactions. No tools/network/private media.'
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
