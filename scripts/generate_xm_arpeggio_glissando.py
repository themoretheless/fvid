"""Owned arpeggio/glissando with literal periods, offsets and reset positions."""
import json,struct
from pathlib import Path
from generate_xm_tone_portamento import generate as wrap,R,S
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-arpeggio-glissando'
def Q(c,degrees):
    l={0:4608,1:4544,4:4352,7:4160,15:3648};a={0:1712,1:1616,4:1359,7:1143,15:720}
    return R(c,[l[x]for x in degrees],[a[x]for x in degrees])
G=R(cell(49,1,effect=14,parameter=0x31),[4608]*4,[1712]*4)
U=R(cell(53,effect=3,parameter=10),[4608,4544,4544,4480],[1712,1712,1616,1616])
M=R(cell(effect=3),[4480,4480,4416,4352],[1616,1525,1525,1440])
def generate():
    cases=[
      ('arp-four',[Q(cell(49,1,parameter=0x47),[0,0,7,4]),S],[0]),
      ('arp-high-only',[Q(cell(49,1,parameter=0x10),[0,0,0,1]),S],[0]),
      ('arp-low-only',[Q(cell(49,1,parameter=0x01),[0,0,1,0]),S],[0]),
      ('arp-max-offset',[Q(cell(49,1,parameter=0xff),[0,0,15,15]),S],[0]),
      ('arp-parameter-change',[Q(cell(49,1,parameter=0x47),[0,0,7,4]),Q(cell(parameter=0x10),[0,0,0,1]),S],[0]),
      ('arp-000-restores',[Q(cell(49,1,parameter=0x47),[0,0,7,4]),R(cell(),[4608]*4,[1712]*4)],[0]),
      ('arp-other-effect-restores',[Q(cell(49,1,parameter=0x47),[0,0,7,4]),R(cell(effect=8,parameter=128),[4608]*4,[1712]*4)],[0]),
      ('arp-held-cycle',[Q(cell(49,1,parameter=0x47)+cell(effect=14,parameter=0xe1),[0,0,7,4,4,0,7,4]),R(cell(),[4608]*4,[1712]*4)],[0]),
      ('arp-speed-one-held',[R(cell(49,1)+cell(effect=15,parameter=1),[4608],[1712]),Q(cell(parameter=0x47)+cell(effect=14,parameter=0xe2),[0,4,4]),R(cell(),[4608],[1712])],[0]),
      ('arp-speed-two',[Q(cell(49,1,parameter=0x47)+cell(effect=15,parameter=2),[0,4]),R(cell(),[4608]*2,[1712]*2)],[0]),
      ('arp-speed-five',[Q(cell(49,1,parameter=0x47)+cell(effect=15,parameter=5),[0,4,0,7,4]),R(cell(),[4608]*5,[1712]*5)],[0]),
      ('arp-speed-six',[Q(cell(49,1,parameter=0x47)+cell(effect=15,parameter=6),[0,7,4,0,7,4]),R(cell(),[4608]*6,[1712]*6)],[0]),
      ('arp-speed-sixteen',[Q(cell(49,1,parameter=0x47)+cell(effect=15,parameter=16),[0,0,7,4,0,7,4,0,7,4,0,7,4,0,7,4]),R(cell(),[4608]*16,[1712]*16)],[0]),
      ('arp-speed-seventeen',[Q(cell(49,1,parameter=0x47)+cell(effect=15,parameter=17),[0,0,0,7,4,0,7,4,0,7,4,0,7,4,0,7,4]),R(cell(),[4608]*17,[1712]*17)],[0]),
      ('arp-speed-thirty-one',[Q(cell(49,1,parameter=0x47)+cell(effect=15,parameter=31),[0,7,7,7,7,7,7,7,7,7,7,7,7,7,7,0,0,7,4,0,7,4,0,7,4,0,7,4,0,7,4]),R(cell(),[4608]*31,[1712]*31)],[0]),
      ('arp-fractional-finetune',[R(cell(49,1,parameter=0x47),[4607.5,4607.5,4160,4352],[1711,1711,1143,1359]),R(cell(),[4607.5]*4,[1711]*4)],[0]),
      ('arp-zero-offset-normalizes-finetune',[R(cell(49,1,parameter=0x10),[4607.5,4607.5,4608,4544],[1711,1711,1712,1616]),R(cell(),[4607.5]*4,[1711]*4)],[0]),
      ('arp-high-clamp-exact-boundary',[R(cell(49,1,parameter=0x10),[1544,1544,1608,1540],[108,108,114,107]),R(cell(),[1544]*4,[108]*4)],[0]),
      ('arp-high-clamp-below-boundary',[R(cell(49,1,parameter=0x10),[1548,1548,1612,1548],[108,108,115,108]),R(cell(),[1548]*4,[108]*4)],[0]),
      ('arp-negative-finetune',[R(cell(49,1,parameter=0x47),[4640,4640,4192,4384],[1762,1762,1176,1399]),R(cell(),[4640]*4,[1762]*4)],[0]),
      ('arp-relative-octave',[R(cell(49,1,parameter=0x47),[3840,3840,3392,3584],[856,856,571,679]),R(cell(),[3840]*4,[856]*4)],[0]),
      ('arp-high-note-clamp',[R(cell(49,1,parameter=0x10),[1536,1536,1600,1540],[107,107,113,107]),R(cell(),[1536]*4,[107]*4)],[0]),
      ('arp-new-note-restores-before-trigger',[Q(cell(49,1,parameter=0x47),[0,0,7,4]),R(cell(53,parameter=0x47),[4352,4352,3904,4096],[1359,1359,907,1078]),R(cell(),[4352]*4,[1359]*4)],[0,4]),
      ('arp-ED-restores-before-delay',[Q(cell(49,1,parameter=0x47),[0,0,7,4]),R(cell(53,effect=14,parameter=0xd2),[4608,4608,4352,4352],[1712,1712,1359,1359]),R(cell(),[4352]*4,[1359]*4)],[0,6]),
      ('arp-keyoff-retains-phase',[Q(cell(49,1,parameter=0x47),[0,0,7,4]),R(cell(97,parameter=0x47),[4608,4608,4160,4352],[1712,1712,1143,1359],[0]*4),R(cell(effect=12,parameter=64),[4608]*4,[1712]*4)],[0]),
      ('arp-independent-channels',[Q(cell(49,1,parameter=0x47)+cell(49,1,parameter=0x10),[0,0,7,4]),R(cell()+cell(),[4608]*4,[1712]*4)],[0]),
      ('gliss-independent-channels',[R(cell(49,1,effect=14,parameter=0x31)+cell(49,1),[4608]*4,[1712]*4),R(cell(53,effect=3,parameter=10)+cell(53,effect=3,parameter=10),U[1],U[2]),R(cell(effect=3)+cell(effect=3),M[1],M[2])],[0]),
      ('gliss-main',[G,U,M,R(cell(effect=3),[4352]*4,[1440,1440,1359,1359])],[0]),
      ('gliss-any-nonzero',[R(cell(49,1,effect=14,parameter=0x3f),[4608]*4,[1712]*4),U,M],[0]),
      ('gliss-toggle-off-keeps-current-output',[G,U,R(cell(effect=14,parameter=0x30),[4480]*4,[1616]*4),R(cell(effect=3),[4480,4448,4408,4368],[1616,1552,1512,1472])],[0]),
      ('gliss-column',[G,R(cell(53,volume=0xf1),[4608,4544,4480,4416],[1712,1616,1616,1525]),R(cell(volume=0xf0),[4416,4352,4352,4352],[1525,1440,1359,1359])],[0]),
      ('gliss-ordinary-slide-unquantized',[G,R(cell(effect=1,parameter=10),[4608,4568,4528,4488],[1712,1672,1632,1592]),R(cell(53,effect=3,parameter=10),[4488,4480,4416,4352],[1592,1525,1525,1440])],[0]),
      ('gliss-arp-uses-real-period',[G,U,R(cell(parameter=0x47),[4480,4488,4032,4224],[1616,1592,1078,1283]),R(cell(),[4488]*4,[1592]*4)],[0]),
      ('gliss-held-ticks',[G,R(cell(53,effect=3,parameter=10)+cell(effect=14,parameter=0xe1),[4608,4544,4544,4480,4480,4416,4352,4352],[1712,1712,1616,1616,1525,1525,1440,1440])],[0]),
    ]
    # S starts a note; after arpeggio it must be a blank row preserving phase.
    for j,(name,rows,resets) in enumerate(cases):
      if name in ['arp-four','arp-high-only','arp-low-only','arp-max-offset','arp-parameter-change']:
        rows[-1]=R(cell(),[4608]*4,[1712]*4)
    files=wrap(cases)
    for name,finetune,relative in [('arp-fractional-finetune',1,0),('arp-negative-finetune',192,0),('arp-relative-octave',0,12),('arp-high-note-clamp',0,48),('arp-zero-offset-normalizes-finetune',1,0),('arp-high-clamp-exact-boundary',240,48),('arp-high-clamp-below-boundary',232,48)]:
      for mode in ['linear','amiga']:
        key=name+'-'+mode+'.xm';b=bytearray(files[key]);at=60+struct.unpack_from('<I',b,60)[0];inst=at+9+struct.unpack_from('<H',b,at+7)[0];sample=inst+struct.unpack_from('<I',b,inst)[0];b[sample+13]=finetune;b[sample+16]=relative;files[key]=bytes(b)
    m=json.loads(files['manifest.json']);m['provenance']='Owned ramp and literal arpeggio/glissando period traces. No external tools/network/private media.'
    for c in m['cases']:
      mode='linear'if c['linear']else'amiga'
      if c['name'].startswith('gliss-main-'):
        key='gliss-main-'+mode+'-off.xm';b=bytearray(files[c['file']]);at=60+struct.unpack_from('<I',b,60)[0];b[at+9+4]=0x30;files[key]=bytes(b)
        c['control_off']=key
        raw=[4608]*4+[4608,4568,4528,4488]+[4488,4448,4408,4368]+[4368,4352,4352,4352]if c['linear']else [1712]*4+[1712,1672,1632,1592]+[1592,1552,1512,1472]+[1472,1432,1392,1359]
        c['off_periods']=raw+[raw[-1]]*8
      if c['name'].startswith('arp-independent-channels-'):
        raw=([4608,4608,4608,4544]+[4608]*4)if c['linear']else([1712,1712,1712,1616]+[1712]*4)
        c['other']=dict(linear=c['linear'],periods=raw+[raw[-1]]*8,volumes=[64]*8+[0]*8,resets=[0])
      if c['name'].startswith('gliss-independent-channels-'):
        raw=[4608]*4+[4608,4568,4528,4488]+[4488,4448,4408,4368]if c['linear']else [1712]*4+[1712,1672,1632,1592]+[1592,1552,1512,1472]
        c['other']=dict(linear=c['linear'],periods=raw+[raw[-1]]*8,volumes=[64]*12+[0]*8,resets=[0])
    files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
