"""Owned E5x scores against literal native period traces; offline."""
import json,struct
from pathlib import Path
from generate_xm_tone_portamento import generate as wrap,R
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-set-finetune'
def Z(c=cell(),l=4608,a=1712,v=None,ticks=4):return R(c,[l]*ticks,[a]*ticks,v)
def generate():
    linear=[4672,4664,4656,4648,4640,4632,4624,4616,4608,4600,4592,4584,4576,4568,4560,4552]
    amiga=[1814,1801,1788,1775,1762,1749,1737,1724,1712,1700,1687,1675,1663,1651,1639,1628]
    cases=[]
    for n,(l,a) in enumerate(zip(linear,amiga)):
      cases.append((f'nibble-{n:x}',[Z(cell(49,1,effect=14,parameter=0x50+n),l,a),Z(l=l,a=a)],[0]))
    tuned=Z(cell(49,1,effect=14,parameter=0x5c),4576,1663)
    up=R(cell(53,effect=3,parameter=16),[4576,4512,4448,4384],[1663,1599,1535,1471])
    more=R(cell(effect=3),[4384,4320,4320,4320],[1471,1407,1343,1320])
    cases += [
      ('no-note-does-not-retune',[tuned,Z(cell(effect=14,parameter=0x50),4576,1663),Z(cell(effect=14,parameter=0x58),4576,1663),up,more],[0]),
      ('instrument-only-does-not-retune',[tuned,Z(cell(instrument=1,effect=14,parameter=0x50),4576,1663),up,more],[0]),
      ('bare-note-overrides',[Z(cell(49,1,0x30),v=[32]*4),Z(cell(49,effect=14,parameter=0x50),4672,1814,[32]*4)],[0,4]),
      ('keyoff-does-not-retune',[Z(cell(49,1,effect=14,parameter=0x50),4672,1814),Z(cell(97,effect=14,parameter=0x5c),4672,1814,[0]*4),Z(cell(effect=12,parameter=64),4672,1814)],[0]),
      ('portamento-uses-active-finetune',[tuned,up,more],[0]),
      ('column-portamento-uses-active-finetune',[tuned,R(cell(53,volume=0xf1,effect=14,parameter=0x50),up[1],up[2]),R(cell(volume=0xf0),more[1],more[2])],[0]),
      ('arpeggio-uses-active-finetune',[tuned,R(cell(parameter=0x47),[4576,4576,4128,4320],[1663,1663,1110,1320]),Z(l=4576,a=1663)],[0]),
      ('glissando-uses-active-finetune',[tuned,Z(cell(effect=14,parameter=0x31),4576,1663),R(cell(53,effect=3,parameter=16),[4576,4512,4448,4384],[1663,1570,1570,1482]),R(cell(effect=3),[4384,4320,4320,4320],[1482,1399,1320,1320])],[0]),
      ('neutral-overrides-sample',[Z(cell(49,1,effect=14,parameter=0x58)),R(cell(53,effect=3,parameter=16),[4608,4544,4480,4416],[1712,1648,1584,1520]),R(cell(effect=3),[4416,4352,4352,4352],[1520,1456,1392,1359])],[0]),
      ('no-note-neutral-keeps-sample-finetune',[Z(cell(49,1),4576,1663),Z(cell(effect=14,parameter=0x58),4576,1663),up,more],[0]),
      ('retrigger-restores-sample-finetune',[Z(cell(49,1,effect=14,parameter=0x58)),R(cell(effect=14,parameter=0x92),[4608,4608,4576,4576],[1712,1712,1663,1663])],[0,6]),
      ('new-note-restores-sample-finetune',[tuned,Z(cell(49)),Z(cell(49,1))],[0,4,8]),
      ('held-row-does-not-retrigger',[Z(cell(49,1,effect=14,parameter=0x5c)+cell(effect=14,parameter=0xe1),4576,1663,ticks=8)],[0]),
      ('speed-one-executes',[Z(cell(49,1,effect=14,parameter=0x5c)+cell(effect=15,parameter=1),4576,1663,ticks=1),Z(cell()+cell(effect=14,parameter=0xe2),4576,1663,ticks=3)],[0]),
      ('relative-octave-and-finetune',[Z(cell(49,1,effect=14,parameter=0x50),3904,907),Z(cell(49,1,effect=14,parameter=0x58),3840,856)],[0,4]),
      ('independent-finetune-channels',[Z(cell(49,1,effect=14,parameter=0x50)+cell(49,1,effect=14,parameter=0x5f),4672,1814),R(cell(53,effect=3,parameter=16)+cell(53,effect=3,parameter=16),[4672,4608,4544,4480],[1814,1750,1686,1622]),R(cell(effect=3)+cell(effect=3),[4480,4416,4416,4416],[1622,1558,1494,1440])],[0]),
    ]
    files=wrap(cases);m=json.loads(files['manifest.json'])
    for c in m['cases']:
      name=c['name'].rsplit('-',1)[0]
      if name in ['neutral-overrides-sample','no-note-neutral-keeps-sample-finetune','retrigger-restores-sample-finetune','relative-octave-and-finetune']:
        b=bytearray(files[c['file']]);at=60+struct.unpack_from('<I',b,60)[0];inst=at+9+struct.unpack_from('<H',b,at+7)[0];sample=inst+struct.unpack_from('<I',b,inst)[0]
        if name=='relative-octave-and-finetune':b[sample+16]=12
        else:b[sample+13]=64
        files[c['file']]=bytes(b)
      if name=='independent-finetune-channels':
        p=[4552]*4+[4552,4488,4424,4360]+[4360,4296,4296,4296]if c['linear']else [1628]*4+[1628,1564,1500,1436]+[1436,1372,1308,1292]
        c['other']=dict(linear=c['linear'],periods=p+[p[-1]]*8,volumes=[64]*12+[0]*8,resets=[0])
    for c in m['cases']:
      if c['name'].startswith(('nibble-0-','nibble-f-')):
        b=bytearray(files[c['file']]);at=60+struct.unpack_from('<I',b,60)[0];b[at+9+3]=b[at+9+4]=0;key=c['file'].replace('.xm','-off.xm');files[key]=bytes(b);c['control_off']=key
    m['provenance']='Owned ramp and literal E5x period/reset traces; all sixteen nibbles. No tools/network/private media.'
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
