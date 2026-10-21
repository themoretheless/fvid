"""Owned instrument auto-vibrato with explicit final pitch traces; offline."""
import json,struct
from pathlib import Path
from generate_xm_tone_portamento import generate as wrap,R
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-auto-vibrato'
def D(c,offsets,vol=None):return R(c,[4608+x for x in offsets],[1712+x for x in offsets],vol)
def generate():
    sine=[-6,-8,-6,0,5,8,5,0];square=[-8,-8,-8,8,8,8,8,-8]
    cases=[];configs={};tails={}
    def add(name,config,rows,resets,tail):cases.append((name,rows,resets));configs[name]=config;tails[name]=tail
    for name,shape,cycle in [('sine',0,sine),('square',1,square),('ramp-up',2,[2,4,6,-8,-6,-4,-2,0]),('ramp-down',3,[-2,-4,-6,-8,6,4,2,0]),('unknown-shape-is-sine',255,sine)]:
      add(name,(shape,0,8,32),[D(cell(49,1),cycle[:4]),D(cell(),cycle[4:])],[0],cycle)
    add('sweep-four',(1,4,8,32),[D(cell(49,1),[-2,-4,-6,8]),D(cell(),[8,8,8,-8])],[0],square)
    add('sweep-equality-keyoff-step',(1,4,8,32),[D(cell(49,1),[-2,-4,-6,8])],[0],[2,2,2,-2,-2,-2,-2,2])
    add('sweep-floor-and-cap',(1,5,8,32),[D(cell(49,1),[-2,-4,-5,6]),D(cell(),[7,8,8,-8])],[0],square)
    add('keyoff-volume-restore',(1,4,8,32),[D(cell(49,1),[-2,-4,-6,8]),D(cell(97),[2,2,2,-2],[0]*4),D(cell(effect=12,parameter=64),[-2,-2,-2,2])],[0],[2,2,2,-2,-2,-2,-2,2])
    add('instrument-versus-bare-note',(1,0,8,32),[D(cell(49,1),square[:4]),D(cell(49,1),square[:4]),D(cell(49),square[4:])],[0,4,8],square)
    add('instrument-only-resets-phase',(1,0,8,32),[D(cell(49,1),square[:4]),D(cell(instrument=1),square[:4])],[0],square[4:]+square[:4])
    add('E92-resets-phase',(1,0,8,32),[D(cell(49,1),square[:4]),D(cell(effect=14,parameter=0x92),[8,8,-8,-8]),D(cell(),[-8,8,8,8])],[0,6],[8,-8,-8,-8,-8,8,8,8])
    add('ED2-resets-phase',(1,0,8,32),[D(cell(49,1),square[:4]),D(cell(effect=14,parameter=0xd2),[8,8,-8,-8]),D(cell(),[-8,8,8,8])],[0,6],[8,-8,-8,-8,-8,8,8,8])
    add('E90-keeps-phase',(1,0,8,32),[D(cell(49,1),square[:4]),D(cell(effect=14,parameter=0x90),square[4:]),D(cell(),square[:4])],[0,4],square[4:]+square[:4])
    add('held-cycle',(1,0,8,32),[D(cell(49,1)+cell(effect=14,parameter=0xe1),square)],[0],square)
    add('speed-one-held',(1,0,8,32),[D(cell(49,1)+cell(effect=15,parameter=1),[-8]),D(cell()+cell(effect=14,parameter=0xe2),[-8,-8,8])],[0],square[4:]+square[:4])
    add('manual-vibrato-layer',(1,0,8,32),[D(cell(49,1,effect=4,parameter=0x44),[-8,-8,4,30]),D(cell(effect=4),[30,37,39,21]),D(cell(),square[:4])],[0],square[4:]+square[:4])
    add('arpeggio-layer',(1,0,8,32),[R(cell(49,1,parameter=0x47),[4600,4600,4152,4360],[1704,1704,1135,1367]),D(cell(),square[4:])],[0],square)
    add('pitch-slide-layer',(1,0,8,32),[R(cell(49,1,effect=1,parameter=16),[4600,4536,4472,4424],[1704,1640,1576,1528]),R(cell(),[4424,4424,4424,4408],[1528,1528,1528,1512])],[0],[-200,-200,-200,-184,-184,-184,-184,-200])
    add('zero-rate-square',(1,0,8,0),[D(cell(49,1),[-8]*4),D(cell(),[-8]*4)],[0],[-8]*8)
    add('zero-depth',(1,0,0,32),[D(cell(49,1),[0]*4),D(cell(),[0]*4)],[0],[0]*8)
    add('half-cycle-rate',(1,0,8,128),[D(cell(49,1),[8,-8,8,-8]),D(cell(),[8,-8,8,-8])],[0],[8,-8,8,-8,8,-8,8,-8])
    add('instrument-selection-and-active-amplitude',(1,0,8,32),[D(cell(49,1),square[:4]),D(cell(instrument=2),square[:4]),D(cell(49),[8,0,-8,0]),D(cell(49,2),[-4,0,4,0])],[0,8,12],[-4,0,4,0,-4,0,4,0])
    add('independent-instrument-channels',(1,0,8,32),[D(cell(49,1)+cell(49,2),square[:4]),D(cell()+cell(),square[4:])],[0],square)
    add('R81-keeps-auto-phase',(1,0,8,32),[D(cell(49,1),square[:4]),D(cell(effect=27,parameter=0x81),square[4:]),D(cell(),square[:4])],[0,4,5,6,7],square[4:]+square[:4])
    add('manual-no-reset-flags-do-not-hold-auto',(1,0,8,32),[D(cell(49,1,effect=14,parameter=0x44),square[:4]),D(cell(49,1,effect=14,parameter=0x74),square[:4])],[0,4],square[4:]+square[:4])
    add('delayed-first-note-activates-clock',(0,0,8,32),[R(cell(49,1,effect=14,parameter=0xd2),[0,0,4602,4600],[0,0,1706,1704],[0,0,64,64]),D(cell(),[-6,0,5,8])],[2],[5,0,-6,-8,-6,0,5,8])
    files=wrap(cases);m=json.loads(files['manifest.json'])
    for c in m['cases']:
      name=c['name'].rsplit('-',1)[0];config=configs[name];b=bytearray(files[c['file']]);at=60+struct.unpack_from('<I',b,60)[0];inst=at+9+struct.unpack_from('<H',b,at+7)[0];b[inst+235:inst+239]=bytes(config);files[c['file']]=bytes(b)
      base=4608 if c['linear']else 1712;c['periods'][-8:]=[base+x for x in tails[name]];c['auto_vibrato']=list(config)
      if name in ['instrument-selection-and-active-amplitude','independent-instrument-channels']:
        b=bytearray(files[c['file']]);other=bytearray(b[inst:]);second=(0,0,4 if name=='instrument-selection-and-active-amplitude'else 8,64 if name=='instrument-selection-and-active-amplitude'else 32);other[235:239]=bytes(second);struct.pack_into('<H',b,72,2);b+=other;files[c['file']]=bytes(b);c['second_auto_vibrato']=list(second)
      if name=='independent-instrument-channels':
        c['other']=dict(linear=c['linear'],periods=[base+x for x in sine+sine],volumes=[64]*8+[0]*8,resets=[0])
      if name in ['sine','square']:
        off=bytearray(b);off[inst+237]=0;key=c['file'].replace('.xm','-off.xm');files[key]=bytes(off);c['control_off']=key
    m['provenance']='Owned ramp and literal final-pitch/sweep/reset traces; no tools, network or private media.'
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
