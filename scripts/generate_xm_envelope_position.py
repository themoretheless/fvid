"""Owned Lxx with literal post-position gains/pans; no envelope oracle code."""
import json,struct
from pathlib import Path
from generate_xm_envelopes import generate as wrap,Z,E
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-envelope-position'
def generate():
    cases=[];specs={};curve=E([(0,64),(4,32),(8,0)]);s=Z(cell(49,1));normal=[16384,14336,12288,10240];tail=[8192,6144,4096,2048,0,0,0,0]
    def add(name,rows,v=None,p=None,values=None,pans=None,fade=0,fade_values=None,resets=[0]):
      cases.append((name,rows,resets));n=sum(len(row[1])for row in rows)+8
      specs[name]=dict(volume=v,pan=p,fade=fade,values=values if values is not None else [16384]*(n-8)+[0]*8,pans=pans if pans is not None else [128]*n,fade_values=fade_values if fade_values is not None else [32768]*n)
    add('mid-segment',[Z(cell(49,1,effect=21,parameter=2))],v=curve,values=[12288,10240,8192,6144,4096,2048]+[0]*6)
    add('zero-position',[Z(cell(49,1,effect=21))],v=curve,values=normal+tail)
    add('exact-point',[Z(cell(49,1,effect=21,parameter=4))],v=curve,values=[8192,6144,4096,2048]+[0]*8)
    add('exact-last-point',[Z(cell(49,1,effect=21,parameter=8))],v=curve,values=[0]*12)
    add('beyond-last-zero',[Z(cell(49,1,effect=21,parameter=255))],v=curve,values=[0]*12)
    add('beyond-last-nonzero',[Z(cell(49,1,effect=21,parameter=255))],v=E([(0,64),(4,32)]),values=[8192]*12)
    add('single-point',[Z(cell(49,1,effect=21,parameter=255))],v=E([(0,32)]),values=[8192]*12)
    add('jump-back-no-sample-restart',[s,Z(cell(effect=21))],v=curve,values=normal+normal+tail)
    add('jump-forward',[s,Z(cell(effect=21,parameter=6))],v=curve,values=normal+[4096,2048,0,0]+[0]*8)
    add('zero-does-not-reuse-memory',[s,Z(cell(effect=21,parameter=2)),Z(cell(effect=21))],v=curve,values=normal+[12288,10240,8192,6144]+normal+tail)
    add('loop-interior',[Z(cell(49,1,effect=21,parameter=2))],v=E([(0,64),(2,32),(4,0)],5,start=0,end=2),values=[8192,4096,16384,12288]*3)
    add('loop-end-jumps-start',[Z(cell(49,1,effect=21,parameter=4))],v=E([(0,64),(2,32),(4,0)],5,start=0,end=2),values=[16384,12288,8192,4096]*3)
    add('sustain-node-release',[Z(cell(49,1,effect=21,parameter=2))],v=E([(0,64),(2,32),(4,0)],3,1),values=[8192]*5+[4096]+[0]*6)
    add('jump-past-sustain',[Z(cell(49,1,effect=21,parameter=3))],v=E([(0,64),(2,32),(4,0)],3,1),values=[4096]+[0]*11)
    add('sustain-loop-end',[Z(cell(49,1,effect=21,parameter=4))],v=E([(0,64),(2,32),(4,0)],7,2,0,2),values=[16384,12288,8192,4096]+[0]*8)
    add('fractional-position-one',[Z(cell(49,1,effect=21,parameter=1))],v=E([(0,64),(3,0)]),values=[10923,5462]+[0]*10)
    add('fractional-position-two',[Z(cell(49,1,effect=21,parameter=2))],v=E([(0,64),(3,0)]),values=[5462]+[0]*11)
    add('before-first-node',[Z(cell(49,1,effect=21,parameter=2))],v=E([(4,64),(8,0)]),values=[16384]*6+[0]*6)
    pan=E([(0,0),(4,32),(8,63)]);pnormal=[0,32,64,96,128,159,190,221]+[252]*4
    add('pan-only-does-not-position',[Z(cell(49,1,effect=21,parameter=4))],p=pan,pans=pnormal)
    add('volume-no-sustain-does-not-position-pan',[Z(cell(49,1,effect=21,parameter=4))],v=E([(0,64)]),p=pan,values=[16384]*12,pans=pnormal)
    add('volume-sustain-positions-pan',[Z(cell(49,1,effect=21,parameter=4))],v=E([(0,64)],3),p=pan,values=[16384]*12,pans=[128,159,190,221]+[252]*8)
    add('disabled-volume-sustain-bit-positions-pan',[Z(cell(49,1,effect=21,parameter=4))],v=E([(0,64)],2),p=pan,pans=[128,159,190,221]+[252]*8)
    add('pan-loop-position',[Z(cell(49,1,effect=21,parameter=4))],v=E([(0,64)],3),p=E([(0,0),(2,32),(4,63)],5,start=0,end=2),values=[16384]*12,pans=[0,64,128,190]*3)
    add('held-cycle-positions-once',[Z(cell(49,1,effect=21)+cell(effect=14,parameter=0xe1),8)],v=curve,values=normal+[8192,6144,4096,2048]+[0]*8)
    add('speed-one-held',[Z(cell(49,1,effect=21,parameter=2)+cell(effect=15,parameter=1),1),Z(cell()+cell(effect=14,parameter=0xe2),3)],v=curve,values=[12288,10240,8192,6144,4096,2048]+[0]*6)
    add('after-keyoff-does-not-rearm-sustain',[s,Z(cell(97)),Z(cell(effect=21))],v=E([(0,64),(2,32),(4,0)],3,1),values=[16384,12288,8192,8192,8192,4096,0,0,16384,12288,8192,4096]+[0]*8)
    add('position-does-not-reset-fade',[s,Z(cell(97)),Z(cell(effect=21))],v=E([(0,64)]),fade=8192,values=[16384]*20,fade_values=[32768]*4+[24576,16384,8192,0]+[0]*12)
    add('new-note-before-position',[s,Z(cell(49,1,effect=21,parameter=2))],v=curve,values=normal+[12288,10240,8192,6144,4096,2048]+[0]*6,resets=[0,4])
    add('active-owner-before-position',[s,Z(cell(instrument=2,effect=21,parameter=2))],v=curve,values=normal+[12288,10240,8192,6144,4096,2048]+[0]*6)
    add('independent-channel-positions',[Z(cell(49,1,effect=21,parameter=2)+cell(49,1,effect=21,parameter=4))],v=curve,values=[12288,10240,8192,6144,4096,2048]+[0]*6)
    add('no-envelope-is-noop',[Z(cell(49,1,effect=21,parameter=255))])
    add('empty-history-is-noop',[Z(cell(effect=21,parameter=2)),s],v=curve,values=[0]*4+normal+tail,resets=[4])
    files=wrap(cases,specs);m=json.loads(files['manifest.json'])
    for c in m['cases']:
      if c['name'].startswith('active-owner-before-position-'):
        b=bytearray(files[c['file']]);at=60+struct.unpack_from('<I',b,60)[0];inst=at+9+struct.unpack_from('<H',b,at+7)[0];other=bytearray(b[inst:]);other[129:177]=bytes(48);other[225]=1;other[233]=1;struct.pack_into('<H',b,72,2);b+=other;files[c['file']]=bytes(b)
    for c in m['cases']:
      if c['name'].startswith('independent-channel-positions-'):
        base=4608 if c['linear']else 1712;c['other']=dict(linear=c['linear'],periods=[base]*12,gains=[v/65536 for v in [8192,6144,4096,2048]+[0]*8],pans=[128]*12,resets=[0])
      if c['name'].startswith(('mid-segment-','volume-sustain-positions-pan-')):
        b=bytearray(files[c['file']]);at=60+struct.unpack_from('<I',b,60)[0];b[at+9+3]=b[at+9+4]=0;key=c['file'].replace('.xm','-off.xm');files[key]=bytes(b);c['control_off']=key
        c['off_gains']=([v/65536 for v in normal+tail])if c['name'].startswith('mid-segment-')else c['gains']
        c['off_pans']=[128]*12 if c['name'].startswith('mid-segment-')else pnormal
    b=bytearray(files['mid-segment-linear.xm']);at=60+struct.unpack_from('<I',b,60)[0];inst=at+9+struct.unpack_from('<H',b,at+7)[0];b[inst+225]=0;files['unrenderable-empty-shape.xm']=bytes(b);m['unrenderable']=['unrenderable-empty-shape.xm']
    m['provenance']='Owned ramp, literal post-Lxx gains/pans/reset traces, no effect algorithm in expected values. No tools/network/private media.'
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
