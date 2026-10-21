"""Owned XM Txy against independent literal volume writes; offline."""
import json
from pathlib import Path
from generate_xm_volume_slide import generate as wrap
from generate_xm_note_delay import C,H,control
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-tremor'
def V(v):return C(e=12,p=v)
def trace(levels,n=0,i=0):return [C(n,i,e=12,p=levels[0])]+[V(v)for v in levels[1:]]
def generate():
    cases=[]
    for name,param,rows in [
      ('11',0x11,[[64,64,64,0],[0,0,64,64],[64,0,0,64]]),
      ('10',0x10,[[64,64,64,0],[0,64,64,0]]),
      ('01',0x01,[[64,64,0,0],[0,64,0,0]]),
      ('00',0,[[64,64,0,64],[64,0,64,0]]),
      ('21',0x21,[[64]*4,[64,0,0,64],[64,64,64,0]]),
      ('12',0x12,[[64,64,64,0],[0,0,0,64],[64,64,0,0]])]:
        cases.append(('phase-'+name,[(cell(49 if j==0 else 0,1 if j==0 else 0,effect=29,parameter=param if j==0 else 0),trace(v,49 if j==0 else 0,1 if j==0 else 0))for j,v in enumerate(rows)]))
    start=(cell(49,1,effect=29,parameter=0x11),trace([64,64,64,0],49,1))
    cases += [
      ('parameter-change',[start,(cell(effect=29,parameter=0x30),trace([0,0,64,64])),(cell(effect=29),trace([64,64,64,0])),(cell(effect=29),trace([0,64,64,64]))]),
      ('pan-retains-mute',[start,(cell(effect=8,parameter=0),[C(e=8,p=0),H,H,H]),(cell(effect=29),trace([0,0,64,64]))]),
      ('global-retains-mute',[start,(cell(effect=16,parameter=32),[C(e=16,p=32),H,H,H]),(cell(effect=29),trace([0,0,64,64]))]),
      ('column-before-tremor',[(cell(49,1,0x30,29,0x11),trace([32,32,32,0],49,1)),(cell(volume=0x61,effect=29),trace([0,0,30,29]))]),
      ('independent-channels',[(cell(49,1,effect=29,parameter=0x11)+cell(53,1,effect=29,parameter=0x01),list(zip(trace([64,64,64,0],49,1),trace([64,64,0,0],53,1)))),(cell(effect=29)+cell(effect=29),list(zip(trace([0,0,64,64]),trace([0,64,0,0]))))]),
      ('blank-retains-mute',[start,(cell(),trace([0]*4)),(cell(effect=12,parameter=32),trace([32]*4))]),
      ('new-note-retains-mute',[start,(cell(53),trace([0]*4,53)),(cell(effect=12,parameter=64),trace([64]*4))]),
      ('instrument-resets-phase',[start,(cell(53,1),trace([64]*4,53,1)),(cell(effect=29),trace([64,64,64,0]))]),
      ('instrument-only-resets-phase',[start,(cell(instrument=1),trace([64]*4,i=1)),(cell(effect=29),trace([64,64,64,0]))]),
      ('fine-zero-restores-volume',[start,(cell(volume=0x80),trace([64]*4)),(cell(effect=29),trace([64,0,64,64]))]),
      ('slide-zero-restores-volume',[start,(cell(effect=10),trace([0,64,64,64]))]),
      ('E92-resets-phase',[start,(cell(effect=14,parameter=0x92),[V(0),H,C(49),H]),(cell(effect=29),trace([0,64,64,0]))]),
      ('E90-keeps-phase',[start,(cell(effect=14,parameter=0x90),[C(49),H,H,H]),(cell(effect=29),trace([0,0,64,64]))]),
      ('ED2-resets-phase',[start,(cell(effect=14,parameter=0xd2),[V(0),H,C(49),H]),(cell(effect=29),trace([0,64,64,0]))]),
      ('R81-restores-output-keeps-phase',[start,(cell(effect=27,parameter=0x81),[C(49,e=12,p=64)]*4),(cell(effect=29),trace([64,0,64,64]))]),
      ('held-cycle',[(cell(49,1,effect=29,parameter=0x11)+cell(effect=14,parameter=0xe1),[(e,H)for e in trace([64,64,64,0,0,64,64,0],49,1)])]),
      ('maximum-phases',[(cell(49,1,effect=29,parameter=0xff)+cell(effect=14,parameter=0xef),[(e,H)for e in trace([64]*17+[0]*16+[64]*16+[0]*15,49,1)])]),
      ('speed-one-does-not-store',[(cell(49,1)+cell(effect=15,parameter=1),[(C(49,1),H)]),(cell(effect=29,parameter=0x11)+cell(),[(H,H)]),(cell(effect=29)+cell(effect=14,parameter=0xe2),[(e,H)for e in trace([64,64,0])])]),
    ]
    files=wrap(cases,control_tick=control)
    m=json.loads(files['manifest.json'])
    for c in m['cases']:c['events']=c.pop('volumes')
    m['provenance']='Owned Txy literal Cxx tick controls; no tools, network or private media.'
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
