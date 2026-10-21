"""Owned XM Pxy and column Dx/Ex versus literal 8xx tick scores; offline."""
import json
from pathlib import Path
from generate_xm_volume_slide import generate as wrap
from generate_xm_volume_lifetime_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-panning-slide'
def generate():
    cases=[
      ('P-left',[(cell(49,1,effect=0x19,parameter=4),[128,124,120,116])]),
      ('P-right',[(cell(49,1,effect=0x19,parameter=0x30),[128,131,134,137])]),
      ('P-high-nibble-wins',[(cell(49,1,effect=0x19,parameter=0x23),[128,130,132,134])]),
      ('P-clamp-left',[(cell(49,1,effect=8,parameter=3),[3]*4),(cell(effect=0x19,parameter=15),[3,0,0,0])]),
      ('P-clamp-right',[(cell(49,1,effect=8,parameter=250),[250]*4),(cell(effect=0x19,parameter=0xf0),[250,255,255,255])]),
      ('P-memory',[(cell(49,1,effect=0x19,parameter=3),[128,125,122,119]),(cell(),[119]*4),(cell(effect=0x19),[119,116,113,110])]),
      ('P-empty-memory',[(cell(49,1,effect=0x19),[128]*4)]),
      ('set-retains-memory',[(cell(49,1,effect=0x19,parameter=3),[128,125,122,119]),(cell(effect=8,parameter=32),[32]*4),(cell(effect=0x19),[32,29,26,23])]),
      ('P-speed-one-no-store',[(cell(49,1,effect=0x19,parameter=4),[128,124,120,116]),(cell(effect=15,parameter=1),[116]),(cell(effect=0x19,parameter=2),[116]),(cell(effect=15,parameter=4),[116]*4),(cell(effect=0x19),[116,112,108,104])]),
      ('P-retrigger-memory',[(cell(49,1,effect=0x19,parameter=4),[128,124,120,116]),(cell(50,1,effect=0x19),[128,124,120,116])]),
      ('P-rewind-empty',[(cell(49,1,effect=0x19),[128]*4),(cell(effect=0x19,parameter=3),[128,125,122,119])]),
      ('P-silent-memory',[(cell(effect=0x19,parameter=3),[0]*4),(cell(49,1,effect=0x19),[128,125,122,119])]),
      ('P-two-channels',[(cell(49,1,effect=0x19,parameter=2)+cell(50,1,effect=0x19,parameter=3),[(128,128),(126,125),(124,122),(122,119)]),(cell(effect=0x19)+cell(effect=0x19),[(122,119),(120,116),(118,113),(116,110)])]),
      ('P-direction-memory',[(cell(49,1,effect=0x19,parameter=2),[128,126,124,122]),(cell(effect=0x19,parameter=0x30),[122,125,128,131]),(cell(effect=0x19),[131,134,137,140])]),
      ('column-left',[(cell(49,1,volume=0xd4),[128,124,120,116])]),
      ('column-right',[(cell(49,1,volume=0xe3),[128,131,134,137])]),
      ('D0-FT2-quirk',[(cell(49,1,volume=0xd0),[128,0,0,0])]),
      ('E0-no-change',[(cell(49,1,volume=0xe0),[128]*4)]),
      ('column-no-P-memory',[(cell(49,1,effect=0x19,parameter=3),[128,125,122,119]),(cell(volume=0xd4),[119,115,111,107]),(cell(effect=0x19),[107,104,101,98])]),
      ('column-before-P-right',[(cell(49,1,effect=8,parameter=3),[3]*4),(cell(volume=0xdf,effect=0x19,parameter=0x10),[3,1,1,1])]),
      ('column-before-P-left',[(cell(49,1,effect=8,parameter=250),[250]*4),(cell(volume=0xef,effect=0x19,parameter=1),[250,254,254,254])]),
      ('Cx-tick-zero',[(cell(49,1,volume=0xc3,effect=0x19,parameter=4),[48,44,40,36])]),
      ('column-speed-one',[(cell(49,1,volume=0xdf,effect=15,parameter=1),[128])]),
      ('A-no-P-memory',[(cell(49,1,effect=0x19,parameter=2),[128,126,124,122]),(cell(effect=0xa,parameter=0x10),[122]*4),(cell(effect=0x19),[122,120,118,116])]),
    ]
    files=wrap(cases, control_effect=8)
    manifest=json.loads(files['manifest.json'])
    for case in manifest['cases']:case['pans']=case.pop('volumes')
    manifest['provenance']='Owned signed16loop and literal independent 8xx pan ticks, including FT2 D0 quirk. No tools/network/private media.'
    files['manifest.json']=(json.dumps(manifest,indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
