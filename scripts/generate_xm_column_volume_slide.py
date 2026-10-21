"""Owned XM column slides and literal independent Cxx controls; offline."""
from generate_xm_volume_slide import generate as wrap
from generate_xm_volume_lifetime_regressions import cell
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-column-volume-slide'
def generate():
    cases=[
      ('column-down',[(cell(49,1,0x64),[64,60,56,52])]),
      ('column-up',[(cell(49,1,0x10),[0]*4),(cell(volume=0x74),[0,4,8,12])]),
      ('zero-does-not-recall',[(cell(49,1,0x64),[64,60,56,52]),(cell(volume=0x60),[52]*4)]),
      ('clamp-low',[(cell(49,1,0x13),[3]*4),(cell(volume=0x6f),[3,0,0,0])]),
      ('clamp-high',[(cell(49,1,0x7f),[64]*4)]),
      ('fine-down-tick-zero',[(cell(49,1,0x83),[61]*4)]),
      ('fine-up-tick-zero',[(cell(49,1,0x30),[32]*4),(cell(volume=0x93),[35]*4)]),
      ('column-before-A-up',[(cell(49,1,0x13),[3]*4),(cell(volume=0x6f,effect=0xa,parameter=0x10),[3,1,1,1])]),
      ('column-before-A-down',[(cell(49,1,0x4c),[60]*4),(cell(volume=0x7f,effect=0xa,parameter=1),[60,63,63,63])]),
      ('Cxx-then-column',[(cell(49,1,0x64,effect=0xc,parameter=32),[32,28,24,20])]),
      ('column-before-cut',[(cell(49,1,0x10),[0]*4),(cell(volume=0x74,effect=14,parameter=0xc2),[0,4,0,4])]),
      ('column-before-key-off',[(cell(49,1,0x10),[0]*4),(cell(volume=0x74,effect=0x14,parameter=2),[0,4,0,4])]),
      ('column-does-not-write-A-memory',[(cell(49,1,0x30,0xa,3),[32,29,26,23]),(cell(volume=0x64),[23,19,15,11]),(cell(effect=0xa),[11,8,5,2])]),
      ('speed-one',[(cell(49,1,0x6f,effect=15,parameter=1),[64])]),
      ('two-channels',[(cell(49,1,0x50)+cell(50,1,0x20),[(64,16)]*4),(cell(volume=0x64)+cell(volume=0x73),[(64,16),(60,19),(56,22),(52,25)])]),
    ]
    return wrap(cases)
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
