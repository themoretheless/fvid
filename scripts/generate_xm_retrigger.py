"""Owned XM E9x against literal tick-note control scores; offline."""
import json,struct
from pathlib import Path
from generate_xm_volume_slide import generate as wrap
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-retrigger'
def control(c,tick,notes):
    out=[]
    for ch,note in enumerate(notes):
        v=c[ch*5:ch*5+5]
        effect,param=(v[3],v[4]) if tick==0 and v[3] not in (14,15) else (0,0)
        out.append(cell(note,v[1] if tick==0 else 0,v[2] if tick==0 else 0,effect,param))
    return b''.join(out)
def generate():
    cases=[
      ('every-tick',[(cell(49,1,effect=14,parameter=0x91),[49,49,49,49])]),
      ('interval-two',[(cell(49,1,effect=14,parameter=0x92),[49,0,49,0])]),
      ('interval-three',[(cell(49,1,effect=14,parameter=0x93),[49,0,0,49])]),
      ('interval-beyond-row',[(cell(49,1,effect=14,parameter=0x9f),[49,0,0,0])]),
      ('row-zero-is-not-repeat',[(cell(49,1),[49,0,0,0]),(cell(effect=14,parameter=0x92),[0,0,49,0])]),
      ('zero-restarts-on-row',[(cell(49,1),[49,0,0,0]),(cell(effect=14,parameter=0x90),[49,0,0,0])]),
      ('zero-has-no-interval-memory',[(cell(49,1,effect=14,parameter=0x92),[49,0,49,0]),(cell(effect=14,parameter=0x90),[49,0,0,0])]),
      ('zero-explicit-new-note',[(cell(49,1),[49,0,0,0]),(cell(50,effect=14,parameter=0x90),[50,0,0,0])]),
      ('zero-empty',[(cell(effect=14,parameter=0x90),[0]*4),(cell(49,1),[49,0,0,0])]),
      ('nonzero-empty',[(cell(effect=14,parameter=0x91),[0]*4),(cell(49,1),[49,0,0,0])]),
      ('speed-one',[(cell(49,1,effect=15,parameter=1),[49]),(cell(effect=14,parameter=0x91),[0])]),
      ('volume-retained',[(cell(49,1,0x30),[49,0,0,0]),(cell(effect=14,parameter=0x91),[0,49,49,49])]),
      ('pan-retained',[(cell(49,1,effect=8,parameter=12),[49,0,0,0]),(cell(effect=14,parameter=0x92),[0,0,49,0])]),
      ('offset-not-reused',[(cell(49,1,effect=9,parameter=1),[49,0,0,0]),(cell(effect=14,parameter=0x91),[0,49,49,49])]),
      ('unlooped-revival',[(cell(49,1,effect=14,parameter=0x91),[49,49,49,49])]),
      ('ping-pong-direction',[(cell(49,1,effect=14,parameter=0x92),[49,0,49,0]),(cell(effect=14,parameter=0x92),[0,0,49,0])]),
      ('two-channels',[(cell(49,1,effect=14,parameter=0x92)+cell(50,1,effect=14,parameter=0x93),[(49,50),(0,0),(49,0),(0,50)]),(cell(effect=14,parameter=0x92)+cell(effect=14,parameter=0x93),[(0,0),(0,0),(49,0),(0,50)])]),
      ('zero-key-off',[(cell(49,1),[49,0,0,0]),(cell(97,effect=14,parameter=0x90),[97,0,0,0])]),
      ('instrument-only-repeat',[(cell(49,1,0x30),[49,0,0,0]),(cell(instrument=1,effect=14,parameter=0x92),[0,0,49,0])]),
      ('new-pitch-repeat',[(cell(49,1),[49,0,0,0]),(cell(53,effect=14,parameter=0x91),[53,53,53,53])]),
      ('selected-instrument-repeat',[(cell(49,1,0x30),[49,0,0,0]),(cell(instrument=2,effect=14,parameter=0x92),[0,0,49,0])]),
      ('selected-instrument-zero',[(cell(49,1),[49,0,0,0]),(cell(instrument=2,effect=14,parameter=0x90),[49,0,0,0])]),
    ]
    files=wrap(cases,control_tick=control)
    for name,mode in [('unlooped-revival',0),('ping-pong-direction',2)]:
        for suffix in ['', '-ticks']:
            key=name+suffix+'.xm';b=bytearray(files[key]);p=60+struct.unpack_from('<I',b,60)[0]
            instrument=p+9+struct.unpack_from('<H',b,p+7)[0]
            sample=instrument+struct.unpack_from('<I',b,instrument)[0]
            b[sample+14]=16+mode;files[key]=bytes(b)
    for name in ['selected-instrument-repeat','selected-instrument-zero']:
        for suffix in ['', '-ticks']:
            key=name+suffix+'.xm';b=bytearray(files[key]);p=60+struct.unpack_from('<I',b,60)[0]
            at=p+9+struct.unpack_from('<H',b,p+7)[0]
            other=bytearray(b[at:]);sample=struct.unpack_from('<I',other,0)[0]
            other[sample+16]=12 # The selected instrument plays one octave up.
            struct.pack_into('<H',b,72,2);b+=other;files[key]=bytes(b)
    m=json.loads(files['manifest.json'])
    for c in m['cases']:c['notes']=c.pop('volumes')
    m['provenance']='Owned signed16loop/unlooped/ping-pong sample and literal tick-note restart controls. No codec tools/network/private media.'
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
