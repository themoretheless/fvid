"""Owned XM EAx/EBx with literal independent Cxx tick controls; offline."""
from generate_xm_volume_slide import generate as wrap
from generate_xm_volume_lifetime_regressions import cell
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-fine-volume-slide'
def generate():
    cases=[
      ('up-memory',[(cell(49,1,0x30,14,0xa3),[35]*4),(cell(effect=14,parameter=0xa0),[38]*4)]),
      ('down-memory',[(cell(49,1,0x30,14,0xb3),[29]*4),(cell(effect=14,parameter=0xb0),[26]*4)]),
      ('direction-memory',[(cell(49,1,0x30,14,0xa2),[34]*4),(cell(effect=14,parameter=0xb5),[29]*4),(cell(effect=14,parameter=0xa0),[31]*4),(cell(effect=14,parameter=0xb0),[26]*4)]),
      ('empty-memories',[(cell(49,1,0x30,14,0xa0),[32]*4),(cell(effect=14,parameter=0xb0),[32]*4)]),
      ('clamp-high',[(cell(49,1,0x4c,14,0xaf),[64]*4)]),
      ('clamp-low',[(cell(49,1,0x13,14,0xbf),[0]*4)]),
      ('column-before-up',[(cell(49,1,0x13),[3]*4),(cell(volume=0x8f,effect=14,parameter=0xa1),[1]*4)]),
      ('column-before-down',[(cell(49,1,0x4c),[60]*4),(cell(volume=0x9f,effect=14,parameter=0xb1),[63]*4)]),
      ('ordinary-column-after-fine',[(cell(49,1,0x30),[32]*4),(cell(volume=0x64,effect=14,parameter=0xa3),[35,31,27,23])]),
      ('A-memory-independent',[(cell(49,1,0x30,0xa,3),[32,29,26,23]),(cell(effect=14,parameter=0xa5),[28]*4),(cell(effect=0xa),[28,25,22,19])]),
      ('column-does-not-store',[(cell(49,1,0x30,14,0xa2),[34]*4),(cell(volume=0x95),[39]*4),(cell(effect=14,parameter=0xa0),[41]*4)]),
      ('retrigger-memory',[(cell(49,1,0x30,14,0xa3),[35]*4),(cell(50,1,0x30,14,0xa0),[35]*4)]),
      ('channel-local',[(cell(49,1,0x30,14,0xa2)+cell(50,1,0x20,14,0xa5),[(34,21)]*4),(cell(effect=14,parameter=0xa0)+cell(effect=14,parameter=0xa0),[(36,26)]*4)]),
      ('speed-one-stores',[(cell(49,1,0x30,15,1),[32]),(cell(effect=14,parameter=0xa3),[35]),(cell(effect=14,parameter=0xa0),[38])]),
    ]
    return wrap(cases)
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
