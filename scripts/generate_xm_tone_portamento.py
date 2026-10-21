"""Owned tone portamento scores with literal independent period/volume traces."""
import json,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime,cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-tone-portamento'
def R(c,l,a,v=None):return(c,l,a,v if v is not None else [64]*len(l))
S=R(cell(49,1),[4608]*4,[1712]*4)
U=R(cell(53,effect=3,parameter=16),[4608,4544,4480,4416],[1712,1648,1584,1520])
M=R(cell(effect=3),[4416,4352,4352,4352],[1520,1456,1392,1359])
def generate(cases_override=None):
    cases=[
      ('empty-history-keeps-silence',[R(cell(53,1,effect=3,parameter=16),[0,64,128,192],[0,64,128,192],[0]*4),S,R(cell(effect=3),[4608,4352,4352,4352],[1712,1359,1359,1359])],[4]),
      ('column-E90-does-not-store-speed',[S,U,R(cell(volume=0xf2,effect=14,parameter=0x90),U[1],U[2])],[0,8]),
      ('column-ED-does-not-store-speed',[S,U,R(cell(volume=0xf2,effect=14,parameter=0xd2),[4416,4352,4608,4352],[1520,1456,1712,1648])],[0,10]),
      ('column-K00-preparation-precedes-keyoff',[S,U,R(cell(volume=0xf1,effect=20),M[1],M[2])],[0]),
      ('column-before-pitch-slide',[S,R(cell(53,volume=0xf1,effect=1,parameter=1),[4608,4540,4472,4404],[1712,1644,1576,1508]),R(cell(effect=3),[4404,4352,4352,4352],[1508,1444,1380,1359])],[0]),
      ('independent-channel-targets-and-speed',[R(cell(49,1)+cell(49,1),[4608]*4,[1712]*4),R(cell(53,effect=3,parameter=16)+cell(45,effect=3,parameter=8),U[1],U[2]),R(cell(effect=3)+cell(effect=3),M[1],M[2]),R(cell(effect=3)+cell(effect=3),[4352]*4,[1359]*4)],[0]),
      ('up',[S,U,M,R(cell(effect=3),[4352]*4,[1359]*4)],[0]),
      ('down',[S,R(cell(45,effect=3,parameter=16),[4608,4672,4736,4800],[1712,1776,1840,1904]),R(cell(effect=3),[4800,4864,4864,4864],[1904,1968,2032,2096]),R(cell(effect=3),[4864]*4,[2096,2157,2157,2157])],[0]),
      ('empty-speed',[S,R(cell(53,effect=3),[4608]*4,[1712]*4),R(cell(effect=3),[4608]*4,[1712]*4)],[0]),
      ('store-without-target',[S,R(cell(effect=3,parameter=16),[4608]*4,[1712]*4),R(cell(53,effect=3),U[1],U[2]),M],[0]),
      ('blank-pauses-slide',[S,U,R(cell(),[4416]*4,[1520]*4),M],[0]),
      ('column-speed',[S,R(cell(53,volume=0xf1),U[1],U[2]),R(cell(volume=0xf0),M[1],M[2])],[0]),
      ('column-reuses-main-memory',[S,U,R(cell(volume=0xf0),M[1],M[2])],[0]),
      ('main-reuses-column-memory',[S,R(cell(53,volume=0xf1),U[1],U[2]),M],[0]),
      ('column-wins-speed-and-double-step',[S,R(cell(53,volume=0xf2,effect=3,parameter=1),[4608,4352,4352,4352],[1712,1456,1359,1359])],[0]),
      ('column-zero-double-step',[S,U,R(cell(57,volume=0xf0,effect=3,parameter=1),[4416,4288,4160,4096],[1520,1392,1264,1136]),R(cell(effect=3),[4096]*4,[1136,1078,1078,1078])],[0]),
      ('combined-volume-slide',[S,U,R(cell(effect=5,parameter=4),M[1],M[2],[64,60,56,52]),R(cell(effect=5),[4352]*4,[1359]*4,[52,48,44,40])],[0]),
      ('combined-shares-A-memory',[S,R(cell(effect=10,parameter=4),[4608]*4,[1712]*4,[64,60,56,52]),R(cell(53,effect=3,parameter=16),U[1],U[2],[52]*4),R(cell(effect=5),M[1],M[2],[52,48,44,40]),R(cell(effect=10),[4352]*4,[1359]*4,[40,36,32,28])],[0]),
      ('combined-column-double-step',[S,U,R(cell(57,volume=0xf0,effect=5,parameter=4),[4416,4288,4160,4096],[1520,1392,1264,1136],[64,60,56,52])],[0]),
      ('new-target-reverses',[S,U,R(cell(45,effect=3),[4416,4480,4544,4608],[1520,1584,1648,1712]),R(cell(effect=3),[4608,4672,4736,4800],[1712,1776,1840,1904])],[0]),
      ('retrigger-remembers-original-note',[S,U,R(cell(effect=14,parameter=0x92),[4416,4416,4608,4608],[1520,1520,1712,1712])],[0,10]),
      ('delayed-recall-remembers-original-note',[S,U,R(cell(effect=14,parameter=0xd2),[4416,4416,4608,4608],[1520,1520,1712,1712])],[0,10]),
      ('instrument-selection-keeps-active-tuning',[S,R(cell(53,2,effect=3,parameter=16),U[1],U[2]),R(cell(effect=14,parameter=0x90),[3840]*4,[856]*4)],[0,8]),
      ('column-full-speed',[S,R(cell(61,volume=0xff),[4608,3840,3840,3840],[1712,856,856,856])],[0]),
      ('equal-target',[S,R(cell(49,effect=3,parameter=255),[4608]*4,[1712]*4)],[0]),
      ('keyoff-and-volume-restore',[S,U,R(cell(97,effect=3),M[1],M[2],[0]*4),R(cell(effect=12,parameter=64),[4352]*4,[1359]*4)],[0]),
      ('held-ticks',[S,R(cell(53,effect=3,parameter=16)+cell(effect=14,parameter=0xe1),[4608,4544,4480,4416,4352,4352,4352,4352],[1712,1648,1584,1520,1456,1392,1359,1359])],[0]),
      ('speed-one-stores',[R(cell(49,1)+cell(effect=15,parameter=1),[4608],[1712]),R(cell(53,effect=3,parameter=16)+cell(),[4608],[1712]),R(cell(effect=3)+cell(effect=15,parameter=4),U[1],U[2]),R(cell(effect=3)+cell(),M[1],M[2])],[0]),
    ]
    if cases_override is not None:cases=cases_override
    source=lifetime();base=source['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0]
    body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=bytearray(base[:at]);struct.pack_into('<HH',header,76,4,125)
    files={'video.mkv':source['video.mkv']};manifest=[]
    for name,rows,resets in cases:
      channels=max(len(c)//5 for c,*_ in rows)
      for linear in [True,False]:
        key=name+('-linear'if linear else '-amiga');h=bytearray(header);struct.pack_into('<HH',h,68,channels,1);struct.pack_into('<H',h,74,int(linear))
        payload=b''.join(c+cell()*(channels-len(c)//5)for c,*_ in rows)+cell(97,effect=15,parameter=4)+cell(97)*(channels-1)
        b=bytes(h)+struct.pack('<IBHH',9,0,len(rows)+1,len(payload))+payload+body
        if name=='instrument-selection-keeps-active-tuning':
          b=bytearray(b);struct.pack_into('<H',b,72,2);other=bytearray(body);sample=struct.unpack_from('<I',other,0)[0];other[sample+16]=12;b+=other;b=bytes(b)
        files[key+'.xm']=b
        periods=[v for row in rows for v in row[1 if linear else 2]];volumes=[v for row in rows for v in row[3]]
        manifest.append(dict(name=key,file=key+'.xm',linear=linear,periods=periods+[periods[-1]]*8,resets=resets,volumes=volumes+[0]*8,release=len(periods)))
        if name=='independent-channel-targets-and-speed':
          second=[4608]*4+[4608,4640,4672,4704]+[4704,4736,4768,4800]+[4800,4832,4864,4864]if linear else [1712]*4+[1712,1744,1776,1808]+[1808,1840,1872,1904]+[1904,1936,1968,2000]
          manifest[-1]['other']=dict(linear=linear,periods=second+[second[-1]]*8,volumes=[64]*len(second)+[0]*8,resets=[0],release=len(second))
    files['manifest.json']=(json.dumps(dict(provenance='Owned ramp, literal tone period/volume traces; no external tools/network/private media.',cases=manifest),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
