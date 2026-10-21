"""Owned vibrato/tremolo against literal modulation traces; offline."""
import json,struct
from pathlib import Path
from generate_xm_tone_portamento import generate as wrap,R
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-vibrato-tremolo'
def D(c,offsets,volumes=None):return R(c,[4608+x for x in offsets],[1712+x for x in offsets],volumes)
def Z(c=cell(),ticks=4,volume=64):return D(c,[0]*ticks,[volume]*ticks)
S=Z(cell(49,1))
V=D(cell(effect=4,parameter=0x88),[0,0,45,63])
T=D(cell(effect=7,parameter=0x88),[0]*4,[32,32,54,63])
A=Z(cell(49,1,0xa8))
def generate():
    cases=[
      ('vib-sine-cycle',[S,D(cell(effect=4,parameter=0x88)+cell(effect=14,parameter=0xe1),[0,0,45,63,45,0,-45,-63]),D(cell(effect=4),[-63,-45,0,45]),Z()],[0]),
      ('vib-ramp-cycle',[Z(cell(49,1,effect=14,parameter=0x41)),D(cell(effect=4,parameter=0x88)+cell(effect=14,parameter=0xe1),[0,0,16,32,48,-63,-47,-31]),D(cell(effect=4),[-31,-15,0,16]),Z()],[0]),
      ('vib-square-cycle',[Z(cell(49,1,effect=14,parameter=0x42)),D(cell(effect=4,parameter=0x88)+cell(effect=14,parameter=0xe1),[0,63,63,63,63,-63,-63,-63]),D(cell(effect=4),[-63,-63,63,63]),Z()],[0]),
      ('vib-wave-three-is-square',[Z(cell(49,1,effect=14,parameter=0x43)),D(cell(effect=4,parameter=0x88),[0,63,63,63]),Z()],[0]),
      ('vib-nibble-memory',[S,V,D(cell(effect=4,parameter=0x40),[63,45,24,0]),D(cell(effect=4,parameter=0x04),[0,-12,-22,-29]),Z()],[0]),
      ('vib-empty-memory',[S,D(cell(effect=4),[0]*4),Z()],[0]),
      ('vib-combined-volume',[Z(cell(49,1,0x30),volume=32),D(cell(effect=4,parameter=0x88),[0,0,45,63],[32]*4),D(cell(effect=6,parameter=4),[63,45,0,-45],[32,28,24,20]),D(cell(effect=10),[0]*4,[20,16,12,8])],[0]),
      ('vib-instrument-resets-phase',[S,V,D(cell(49,1,effect=4),[0,0,45,63]),Z()],[0,8]),
      ('vib-instrument-holds-phase',[Z(cell(49,1,effect=14,parameter=0x44)),V,D(cell(49,1,effect=4),[0,45,0,-45]),Z()],[0,8]),
      ('vib-note-without-instrument-keeps-phase',[S,V,D(cell(49,effect=4),[0,45,0,-45]),Z()],[0,8]),
      ('vib-instrument-only-resets-phase',[S,V,D(cell(instrument=1,effect=4),[63,0,45,63]),Z()],[0]),
      ('vib-E92-resets-phase',[S,V,Z(cell(effect=14,parameter=0x92)),D(cell(effect=4),[0,0,45,63]),Z()],[0,10]),
      ('vib-E90-keeps-phase',[S,V,Z(cell(effect=14,parameter=0x90)),D(cell(effect=4),[0,45,0,-45]),Z()],[0,8]),
      ('vib-ED2-resets-phase',[S,V,Z(cell(effect=14,parameter=0xd2)),D(cell(effect=4),[0,0,45,63]),Z()],[0,10]),
      ('column-speed-and-depth',[A,D(cell(volume=0xb8),[0,0,45,63]),D(cell(),[63]*4),D(cell(volume=0xb0),[63,45,0,-45]),D(cell(),[-45]*4)],[0]),
      ('column-double-vibrato',[A,D(cell(volume=0xb8,effect=4),[0,45,45,-45]),D(cell(effect=4),[-45,-63,-45,0]),Z()],[0]),
      ('column-main-overrides-memory',[A,D(cell(volume=0xb8,effect=4,parameter=0x12),[0,11,13,14]),Z()],[0]),
      ('vib-main-to-column-restores-base',[S,V,D(cell(volume=0xb0),[0,45,0,-45]),D(cell(),[-45]*4)],[0]),
      ('trem-sine-retains-output',[Z(cell(49,1,0x30),volume=32),T,D(cell(effect=7),[0]*4,[63,54,32,10]),D(cell(),[0]*4,[10]*4),Z(cell(effect=12,parameter=32),volume=32)],[0]),
      ('trem-ramp-vibrato-sign-quirk',[Z(cell(49,1,0x30,14,0x71),volume=32),D(cell(effect=7,parameter=0x88)+cell(effect=14,parameter=0xe1),[0]*8,[32,32,40,48,56,32,24,16]),D(cell(effect=7),[0]*4,[16,8,32,40]),Z(cell(effect=12,parameter=32),volume=32)],[0]),
      ('trem-square-clamps',[Z(cell(49,1,0x30,14,0x72),volume=32),D(cell(effect=7,parameter=0x8f)+cell(effect=14,parameter=0xe1),[0]*8,[32,64,64,64,64,0,0,0]),Z(cell(effect=12,parameter=32),volume=32)],[0]),
      ('trem-instrument-resets-phase',[Z(cell(49,1,0x30),volume=32),T,D(cell(49,1,0x30,7),[0]*4,[32,32,54,63]),Z(cell(effect=12,parameter=32),volume=32)],[0,8]),
      ('trem-instrument-holds-phase',[Z(cell(49,1,0x30,14,0x74),volume=32),T,D(cell(49,1,0x30,7),[0]*4,[32,54,32,10]),Z(cell(effect=12,parameter=32),volume=32)],[0,8]),
      ('trem-E92-resets-phase-keeps-output',[Z(cell(49,1,0x30),volume=32),T,D(cell(effect=14,parameter=0x92),[0]*4,[63]*4),D(cell(effect=7),[0]*4,[63,32,54,63]),Z(cell(effect=12,parameter=32),volume=32)],[0,10]),
      ('trem-E90-keeps-phase-output',[Z(cell(49,1,0x30),volume=32),T,D(cell(effect=14,parameter=0x90),[0]*4,[63]*4),D(cell(effect=7),[0]*4,[63,54,32,10]),Z(cell(effect=12,parameter=32),volume=32)],[0,8]),
      ('trem-ED2-resets-phase-keeps-output',[Z(cell(49,1,0x30),volume=32),T,D(cell(effect=14,parameter=0xd2),[0]*4,[63]*4),D(cell(effect=7),[0]*4,[63,32,54,63]),Z(cell(effect=12,parameter=32),volume=32)],[0,10]),
      ('trem-then-tremor',[Z(cell(49,1,0x30),volume=32),T,D(cell(effect=29,parameter=0x11),[0]*4,[63,32,32,0]),Z(cell(effect=12,parameter=32),volume=32)],[0]),
      ('trem-volume-slide-restores-logical',[Z(cell(49,1,0x30),volume=32),T,D(cell(effect=10,parameter=1),[0]*4,[63,31,30,29])],[0]),
      ('vib-full-nibble-cycle',[S,D(cell(effect=4,parameter=0xff)+cell(effect=14,parameter=0xe1),[0,0,118,22,-114,-45,105,66]),D(cell(effect=4),[66,-92,-84,75]),Z()],[0]),
      ('vib-speed-one-does-not-store',[S,V,Z(cell(effect=15,parameter=1),ticks=1),D(cell(effect=4,parameter=0x44),[0]),Z(cell(effect=15,parameter=4)),D(cell(effect=4),[0,45,0,-45]),Z()],[0]),
      ('trem-speed-one-does-not-store',[Z(cell(49,1,0x30),volume=32),T,D(cell(effect=15,parameter=1),[0],[63]),D(cell(effect=7,parameter=0x44),[0],[63]),D(cell(effect=15,parameter=4),[0]*4,[63]*4),D(cell(effect=7),[0]*4,[63,54,32,10]),Z(cell(effect=12,parameter=32),volume=32)],[0]),
      ('trem-nibble-memory',[Z(cell(49,1,0x30),volume=32),T,D(cell(effect=7,parameter=0x40),[0]*4,[63,54,44,32]),D(cell(effect=7,parameter=0x04),[0]*4,[32,26,21,18]),D(cell(effect=7),[0]*4,[18,17,18,21]),Z(cell(effect=12,parameter=32),volume=32)],[0]),
      ('trem-full-nibble-cycle',[Z(cell(49,1,0x30),volume=32),D(cell(effect=7,parameter=0xff)+cell(effect=14,parameter=0xe1),[0]*8,[32,32,64,43,0,10,64,64]),Z(cell(effect=12,parameter=32),volume=32)],[0]),
      ('trem-ramp-negative-vibrato-sign',[Z(cell(49,1,0x30),volume=32),D(cell(effect=4,parameter=0x88)+cell(effect=14,parameter=0xe1),[0,0,45,63,45,0,-45,-63],[32]*8),Z(cell(effect=14,parameter=0x71),volume=32),D(cell(effect=7,parameter=0x88)+cell(effect=14,parameter=0xe1),[0]*8,[32,63,55,47,39,1,9,17]),Z(cell(effect=12,parameter=32),volume=32)],[0]),
      ('trem-R81-restores-volume-keeps-phase',[Z(cell(49,1,0x30),volume=32),T,Z(cell(effect=27,parameter=0x81),volume=32),D(cell(effect=7),[0]*4,[32,54,32,10]),Z(cell(effect=12,parameter=32),volume=32)],[0,8,9,10,11]),
      ('trem-pan-write-keeps-output',[Z(cell(49,1,0x30),volume=32),T,D(cell(effect=8,parameter=128),[0]*4,[63]*4),Z(cell(effect=12,parameter=32),volume=32)],[0]),
      ('tremor-then-tremolo',[Z(cell(49,1,0x30),volume=32),D(cell(effect=29,parameter=0x11),[0]*4,[32,32,32,0]),D(cell(effect=7,parameter=0x88),[0]*4,[0,32,54,63]),Z(cell(effect=12,parameter=32),volume=32)],[0]),
      ('column-A-retrigger-zero-result',[Z(cell(49,1,0x30,27,0x83),volume=32),Z(cell(volume=0xa0,effect=27,parameter=0x83),volume=32),Z(cell(volume=0xa1,effect=27,parameter=0x83),volume=32)],[0,3,6,10]),
      ('independent-vibrato-tremolo-channels',[Z(cell(49,1)+cell(49,1,0x30)),D(cell(effect=4,parameter=0x88)+cell(effect=7,parameter=0x88),[0,0,45,63]),D(cell(effect=4)+cell(effect=7),[63,45,0,-45]),Z(cell()+cell())],[0]),
      ('vib-zero-period-freezes-cursor',[R(cell(49,1,effect=14,parameter=0x42),[95]*4,[29]*4),R(cell(effect=4,parameter=0x8c)+cell(effect=14,parameter=0xe1),[95,190,190,190,190,0,0,0],[29,124,124,124,124,65470,65470,65470]),R(cell(),[95]*4,[29]*4)],[0]),

    ]
    files=wrap(cases)
    m=json.loads(files['manifest.json']);m['provenance']='Owned signed16 loop and literal vibrato/tremolo pitch/volume traces; no tools/network/private media.'
    for c in m['cases']:
      if c['name'].startswith('vib-zero-period-freezes-cursor-'):
        b=bytearray(files[c['file']]);at=60+struct.unpack_from('<I',b,60)[0];inst=at+9+struct.unpack_from('<H',b,at+7)[0];sample=inst+struct.unpack_from('<I',b,inst)[0];b[sample+13]=66;b[sample+16]=70;files[c['file']]=bytes(b)
      if c['name'].startswith(('vib-sine-cycle-','trem-sine-retains-output-')):
        b=bytearray(files[c['file']]);at=60+struct.unpack_from('<I',b,60)[0];count=struct.unpack_from('<H',b,at+7)[0]
        for pos in range(at+9,at+9+count,5):
          if b[pos+3]in[4,6,7]:b[pos+3]=b[pos+4]=0
        key=c['file'].replace('.xm','-off.xm');files[key]=bytes(b);c['control_off']=key
        c['off_periods']=([4608]if c['linear']else[1712])*len(c['periods'])
        c['off_volumes']=[32 if c['name'].startswith('trem-')else 64]*c['release']+[0]*8
      if c['name'].startswith('independent-vibrato-tremolo-channels-'):
        c['other']=dict(linear=c['linear'],periods=([4608]if c['linear']else[1712])*24,volumes=[32]*4+[32,32,54,63]+[63,54,32,10]+[10]*4+[0]*8,resets=[0])
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
