"""Owned envelope/fade scores with literal final tick gains and panning."""
import json,struct
from pathlib import Path
from generate_xm_tone_portamento import generate as wrap,R
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-envelopes'
def Z(c=cell(),ticks=4):return R(c,[4608]*ticks,[1712]*ticks)
def E(points,flags=1,sustain=0,start=0,end=0):return dict(points=points,flags=flags,sustain=sustain,start=start,end=end)
def generate(cases_override=None, specs_override=None):
    cases=[];specs={}
    def add(name,rows,v=None,p=None,fade=0,values=None,pans=None,fade_values=None,resets=[0]):
      cases.append((name,rows,resets));ticks=sum(len(row[1])for row in rows)+8
      specs[name]=dict(volume=v,pan=p,fade=fade,values=values if values is not None else [16384]*(ticks-8)+[0]*8,pans=pans if pans is not None else [128]*ticks,fade_values=fade_values if fade_values is not None else [32768]*ticks)
    s=Z(cell(49,1))
    add('volume-fall',[s,Z()],v=E([(0,64),(4,0)]),values=[16384,12288,8192,4096]+[0]*12)
    add('volume-rise-fall',[s,Z()],v=E([(0,0),(4,64),(8,0)]),values=[0,4096,8192,12288,16384,12288,8192,4096]+[0]*8)
    add('volume-sustain-release',[s,Z(cell(97))],v=E([(0,64),(2,32),(4,0)],3,1),values=[16384,12288,8192,8192,8192,4096,0,0]+[0]*8)
    add('volume-loop',[s,Z()],v=E([(0,64),(2,32),(4,0)],5,start=0,end=2),values=[16384,12288,8192,4096]*4)
    add('volume-loop-sustain-end-keyoff',[s,Z()],v=E([(0,64),(2,32),(4,0)],7,2,0,2),values=[16384,12288,8192,4096]*2+[0]*8)
    add('volume-fixedpoint-truncation',[s,Z()],v=E([(0,64),(3,0)]),values=[16384,10923,5462]+[0]*13)
    add('pan-rise',[s,Z()],p=E([(0,0),(2,32),(4,63)]),pans=[0,64,128,190]+[252]*12)
    add('pan-full-scale-wrap',[s],p=E([(0,64)]),pans=[0]*12)
    add('pan-off-centre',[Z(cell(49,1,effect=8,parameter=64)),Z()],p=E([(0,0),(2,32),(4,64)]),pans=[0,32,64,96]+[128]*12)
    add('fade-quarter',[s,Z(cell(97))],v=E([(0,64)]),fade=8192,values=[16384]*16,fade_values=[32768]*4+[24576,16384,8192,0]+[0]*8)
    add('fade-overflow-clamps',[s,Z(cell(97))],v=E([(0,64)]),fade=65535,values=[16384]*16,fade_values=[32768]*4+[0]*12)
    fall=E([(0,64),(4,0)])
    add('note-instrument-resets-envelope',[s,Z(cell(49,1))],v=fall,values=[16384,12288,8192,4096]*2+[0]*8,resets=[0,4])
    add('bare-note-preserves-envelope',[s,Z(cell(49))],v=fall,values=[16384,12288,8192,4096]+[0]*12,resets=[0,4])
    add('instrument-only-resets-envelope',[s,Z(cell(instrument=1))],v=fall,values=[16384,12288,8192,4096]*2+[0]*8)
    for name,param,reset in [('E92',0x92,6),('ED2',0xd2,6)]:
      add(name+'-resets-envelope',[s,Z(cell(effect=14,parameter=param))],v=fall,values=[16384,12288,8192,4096,0,0,16384,12288,8192,4096]+[0]*6,resets=[0,reset])
    add('E90-keeps-envelope',[s,Z(cell(effect=14,parameter=0x90))],v=fall,values=[16384,12288,8192,4096]+[0]*12,resets=[0,4])
    add('ED-first-note-activates-envelope',[R(cell(49,1,effect=14,parameter=0xd2),[0,0,4608,4608],[0,0,1712,1712]),Z()],v=fall,values=[0,0,16384,12288,8192,4096]+[0]*10,resets=[2])
    add('channel-volume-multiplies-envelope',[Z(cell(49,1,0x30))],v=E([(0,32)]),values=[4096]*12)
    add('slide-multiplies-envelope',[Z(cell(49,1,effect=10,parameter=4)),Z(cell(effect=10))],v=E([(0,32)]),values=[8192,7680,7168,6656,6656,6144,5632,5120]+[5120]*8)
    add('pan-loop',[s,Z()],p=E([(0,0),(2,32),(4,63)],5,start=0,end=2),pans=[0,64,128,190]*4)
    add('pan-sustain-keyoff-reference-quirk',[s,Z(cell(97))],v=E([(0,64)]),p=E([(0,0),(2,32),(4,63)],3,1),values=[16384]*16,pans=[0,64,128,128]+[128]*12)
    add('volume-and-pan',[s,Z()],v=fall,p=E([(0,0),(2,32),(4,63)]),values=[16384,12288,8192,4096]+[0]*12,pans=[0,64,128,190]+[252]*12)
    add('fade-cannot-revive-without-instrument',[s,Z(cell(97)),Z(cell(effect=12,parameter=64)),Z(cell(49,1))],fade=8192,values=[16384]*4+[0]*8+[16384]*4+[0]*8,resets=[0,12])
    add('duplicate-point-ticks-are-bounded',[s],v=E([(0,64),(0,0)]),values=[16384]*4+[0]*8)
    add('scheduled-keyoff-releases-envelope',[s,Z(cell(effect=20,parameter=2))],v=E([(0,64),(2,32),(4,0)],3,1),values=[16384,12288,8192,8192,8192,8192,8192,4096]+[0]*8)
    add('scheduled-cut-overrides-envelope',[s,Z(cell(effect=14,parameter=0xc2))],v=E([(0,64)]),values=[16384]*6+[0]*10)
    add('independent-envelope-owners',[Z(cell(49,1)+cell(49,2)),Z(cell()+cell())],v=fall,values=[16384,12288,8192,4096]+[0]*12)
    if cases_override is not None:cases,specs=cases_override,specs_override
    files=wrap(cases);m=json.loads(files['manifest.json'])
    for c in m['cases']:
      name=c['name'].rsplit('-',1)[0];sp=specs[name];b=bytearray(files[c['file']]);at=60+struct.unpack_from('<I',b,60)[0];inst=at+9+struct.unpack_from('<H',b,at+7)[0]
      for j,key in enumerate(['volume','pan']):
        spec=sp[key];b[inst+129+j*48:inst+177+j*48]=bytes(48);b[inst+225+j]=0;b[inst+233+j]=0
        if spec:
          b[inst+225+j]=len(spec['points']);b[inst+233+j]=spec['flags']
          for n,(x,y)in enumerate(spec['points']):struct.pack_into('<HH',b,inst+129+j*48+n*4,x,y)
          b[inst+227+j*3:inst+230+j*3]=bytes([spec['sustain'],spec['start'],spec['end']])
      struct.pack_into('<H',b,inst+239,sp['fade']);files[c['file']]=bytes(b)
      assert len(c['periods'])==len(sp['values'])==len(sp['pans'])==len(sp['fade_values']),name
      c['gains']=[(v/16384)*(f/32768)*.25 for v,f in zip(sp['values'],sp['fade_values'])];c['pans']=sp['pans'];c['envelopes']=sp
      if name=='independent-envelope-owners':
        b=bytearray(files[c['file']]);other=bytearray(b[inst:]);other[129:177]=bytes(48)
        for n,(x,y)in enumerate([(0,0),(4,64),(8,0)]):struct.pack_into('<HH',other,129+n*4,x,y)
        other[225]=3;other[233]=1;struct.pack_into('<H',b,72,2);b+=other;files[c['file']]=bytes(b)
        base=4608 if c['linear']else 1712
        c['other']=dict(linear=c['linear'],periods=[base]*16,gains=[v/65536 for v in [0,4096,8192,12288,16384,12288,8192,4096]+[0]*8],pans=[128]*16,resets=[0])
      if name in ['volume-fall','pan-rise','fade-quarter']:
        off=bytearray(b)
        if name=='volume-fall':off[inst+233]=0
        elif name=='pan-rise':off[inst+234]=0
        else:struct.pack_into('<H',off,inst+239,0)
        key=c['file'].replace('.xm','-off.xm');files[key]=bytes(off);c['control_off']=key
        c['off_gains']=[.25]*c['release']+([.25]*8 if name=='fade-quarter'else[0]*8)
        c['off_pans']=[128]*len(c['periods'])
    m['unrenderable']=[]
    if 'volume-loop-linear.xm'in files:
      bad=bytearray(files['volume-loop-linear.xm']);at=60+struct.unpack_from('<I',bad,60)[0];inst=at+9+struct.unpack_from('<H',bad,at+7)[0];bad[inst+229]=250;files['unrenderable-loop-index.xm']=bytes(bad)
      m['unrenderable']=['unrenderable-loop-index.xm']
    m['provenance']='Owned ramp and literal final envelope/fade/pan traces. No external tools/network/private media.'
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
