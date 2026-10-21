"""Owned pitch slides with literal period traces, not slide-derived expectations."""
import json,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime,cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-pitch-slide'
def generate():
    source=lifetime();base=source['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0]
    body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=bytearray(base[:at]);struct.pack_into('<HH',header,76,4,125)
    # Each offset is an explicitly authored period relative to C4, one per tick.
    cases=[
      ('up-clamp',[(cell(49,1,effect=1,parameter=255)+cell(effect=14,parameter=0xe1),[0]*8)]),
      ('down-signed-clamp',[(cell(1,1,effect=2,parameter=255)+cell(effect=14,parameter=0xe1),[0]*8)]),
      ('fine-extra-empty-memory',[(cell(49,1,effect=14,parameter=0x10),[0]*4),(cell(effect=14,parameter=0x20),[0]*4),(cell(effect=33,parameter=0x10),[0]*4),(cell(effect=33,parameter=0x20),[0]*4)]),
      ('fractional-finetune',[(cell(49,1,effect=1,parameter=1),[0,-4,-8,-12]),(cell(effect=33,parameter=0x13),[-15]*4)]),
      ('ordinary-up',[(cell(49,1,effect=1,parameter=16),[0,-64,-128,-192]),(cell(),[-192]*4)]),
      ('ordinary-down',[(cell(49,1,effect=2,parameter=16),[0,64,128,192]),(cell(),[192]*4)]),
      ('separate-memories',[(cell(49,1,effect=1,parameter=16),[0,-64,-128,-192]),(cell(effect=2,parameter=4),[-192,-176,-160,-144]),(cell(effect=1),[-144,-208,-272,-336]),(cell(effect=2),[-336,-320,-304,-288])]),
      ('empty-memory',[(cell(49,1,effect=1),[0]*4),(cell(effect=2),[0]*4)]),
      ('note-resets-period-keeps-memory',[(cell(49,1,effect=1,parameter=16),[0,-64,-128,-192]),(cell(49,1,effect=1),[0,-64,-128,-192])]),
      ('instrument-only-preserves-period',[(cell(49,1,effect=1,parameter=16),[0,-64,-128,-192]),(cell(instrument=1),[-192]*4),(cell(effect=1),[-192,-256,-320,-384])]),
      ('fine-up',[(cell(49,1,effect=14,parameter=0x13),[-12]*4),(cell(effect=14,parameter=0x10),[-24]*4)]),
      ('fine-down',[(cell(49,1,effect=14,parameter=0x23),[12]*4),(cell(effect=14,parameter=0x20),[24]*4)]),
      ('extra-up',[(cell(49,1,effect=33,parameter=0x13),[-3]*4),(cell(effect=33,parameter=0x10),[-6]*4)]),
      ('extra-down',[(cell(49,1,effect=33,parameter=0x23),[3]*4),(cell(effect=33,parameter=0x20),[6]*4)]),
      ('six-independent-memories',[(cell(49,1,effect=1,parameter=2),[0,-8,-16,-24]),(cell(effect=2,parameter=3),[-24,-12,0,12]),(cell(effect=14,parameter=0x14),[-4]*4),(cell(effect=14,parameter=0x25),[16]*4),(cell(effect=33,parameter=0x16),[10]*4),(cell(effect=33,parameter=0x27),[17]*4),(cell(effect=1),[17,9,1,-7]),(cell(effect=2),[-7,5,17,29]),(cell(effect=14,parameter=0x10),[13]*4),(cell(effect=14,parameter=0x20),[33]*4),(cell(effect=33,parameter=0x10),[27]*4),(cell(effect=33,parameter=0x20),[34]*4)]),
      ('speed-one-does-not-store',[(cell(49,1,effect=1,parameter=16),[0,-64,-128,-192]),(cell(effect=15,parameter=1),[-192]),(cell(effect=1,parameter=4),[-192]),(cell(effect=15,parameter=4),[-192]*4),(cell(effect=1),[-192,-256,-320,-384])]),
      ('held-ordinary',[(cell(49,1,effect=1,parameter=16)+cell(effect=14,parameter=0xe1),[0,-64,-128,-192,-256,-320,-384,-448])]),
      ('held-fine-does-not-repeat',[(cell(49,1,effect=14,parameter=0x13)+cell(effect=14,parameter=0xe1),[-12]*8)]),
      ('held-extra-does-not-repeat',[(cell(49,1,effect=33,parameter=0x13)+cell(effect=14,parameter=0xe1),[-3]*8)]),
      ('retrigger-restores-note-pitch',[(cell(49,1,effect=1,parameter=16),[0,-64,-128,-192]),(cell(effect=14,parameter=0x92),[-192,-192,0,0])]),
      ('delayed-note-restores-pitch',[(cell(49,1,effect=1,parameter=16),[0,-64,-128,-192]),(cell(effect=14,parameter=0xd2),[-192,-192,0,0])]),
    ]
    files={'video.mkv':source['video.mkv']};manifest=[]
    for name,rows in cases:
      for linear,baseperiod in [(True,4608),(False,1712)]:
        key=name+('-linear'if linear else '-amiga');channels=len(rows[0][0])//5
        h=bytearray(header);struct.pack_into('<H',h,68,channels);struct.pack_into('<H',h,74,int(linear))
        payload=b''.join(c for c,_ in rows)+cell(97,effect=15,parameter=4)+cell(97)*(channels-1)
        files[key+'.xm']=bytes(h)+struct.pack('<IBHH',9,0,len(rows)+1,len(payload))+payload+body
        periods=[baseperiod+d for _,vs in rows for d in vs]
        if name=='up-clamp':periods=[4608,3588,2568,1548,528,1,1,1]if linear else [1712,692,1,1,1,1,1,1]
        if name=='down-signed-clamp':periods=[7680,8700,9720,10740,11760,12780,13800,14820]if linear else [27392,28412,29432,30452,31472,31999,33019,34039]
        if name=='fractional-finetune':
          periods=[v-(0.5 if linear else 1)for v in periods]
          b=bytearray(files[key+'.xm']);sample=at+9+len(payload)+struct.unpack_from('<I',body,0)[0];b[sample+13]=1;files[key+'.xm']=bytes(b)
        if name=='ordinary-up':
          b=bytearray(files[key+'.xm']);b[at+9+3]=0;b[at+9+4]=0;files[key+'-no-slide.xm']=bytes(b)
        resets=[0];tick=0
        for c,vs in rows:
          if tick and c[0]:resets.append(tick)
          if name in ['retrigger-restores-note-pitch','delayed-note-restores-pitch']and tick:resets.append(tick+2)
          tick+=len(vs)
        manifest.append(dict(name=key,file=key+'.xm',linear=linear,periods=periods+[periods[-1]]*8,resets=resets,release=tick))
    files['manifest.json']=(json.dumps(dict(provenance='Owned signed16 loop and literal per-tick periods; no external tools/network/private media.',cases=manifest),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
