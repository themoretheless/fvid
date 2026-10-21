"""Owned XM Hxy versus literal independent Gxx tick scores; offline."""
from generate_xm_volume_slide import generate as wrap
from generate_xm_volume_lifetime_regressions import cell
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-global-volume-slide'
def generate():
    cases=[
      ('down',[(cell(49,1,effect=0x11,parameter=2),[64,62,60,58])]),
      ('up',[(cell(49,1,effect=0x10,parameter=16),[16]*4),(cell(effect=0x11,parameter=0x30),[16,19,22,25])]),
      ('high-nibble-wins',[(cell(49,1,effect=0x10,parameter=32),[32]*4),(cell(effect=0x11,parameter=0x23),[32,34,36,38])]),
      ('clamp-low',[(cell(49,1,effect=0x10,parameter=3),[3]*4),(cell(effect=0x11,parameter=15),[3,0,0,0])]),
      ('clamp-high',[(cell(49,1,effect=0x11,parameter=0xf0),[64]*4)]),
      ('memory',[(cell(49,1,effect=0x11,parameter=3),[64,61,58,55]),(cell(),[55]*4),(cell(effect=0x11),[55,52,49,46])]),
      ('empty-memory',[(cell(49,1,effect=0x11),[64]*4)]),
      ('rewind-clears-memory',[(cell(49,1,effect=0x11),[64]*4),(cell(effect=0x11,parameter=2),[64,62,60,58])]),
      ('zero-restores-phase',[(cell(49,1,effect=0x10,parameter=0),[0]*4),(cell(effect=0x11,parameter=0x10),[0,1,2,3])]),
      ('set-does-not-clear-memory',[(cell(49,1,effect=0x11,parameter=2),[64,62,60,58]),(cell(effect=0x10,parameter=32),[32]*4),(cell(effect=0x11),[32,30,28,26])]),
      ('A-does-not-store-H',[(cell(49,1,effect=0x11,parameter=2),[64,62,60,58]),(cell(effect=0xa,parameter=0x10),[58]*4),(cell(effect=0x11),[58,56,54,52])]),
      ('speed-one-does-not-store',[(cell(49,1,effect=0x11,parameter=4),[64,60,56,52]),(cell(effect=15,parameter=1),[52]),(cell(effect=0x11,parameter=2),[52]),(cell(effect=15,parameter=4),[52]*4),(cell(effect=0x11),[52,48,44,40])]),
      ('silent-channel',[(cell(49,1)+cell(effect=0x11,parameter=3),[(64,64),(61,61),(58,58),(55,55)])]),
      ('channel-order-up-down',[(cell(49,1,effect=0x10,parameter=63)+cell(50,1),[(63,63)]*4),(cell(effect=0x11,parameter=0xf0)+cell(effect=0x11,parameter=2),[(63,63),(62,62),(62,62),(62,62)])]),
      ('channel-order-down-up',[(cell(49,1,effect=0x10,parameter=63)+cell(50,1),[(63,63)]*4),(cell(effect=0x11,parameter=2)+cell(effect=0x11,parameter=0xf0),[(63,63),(64,64),(64,64),(64,64)])]),
      ('channel-local-memory',[(cell(49,1,effect=0x11,parameter=2)+cell(50,1,effect=0x11,parameter=3),[(64,64),(59,59),(54,54),(49,49)]),(cell(effect=0x11)+cell(),[(49,49),(47,47),(45,45),(43,43)]),(cell()+cell(effect=0x11),[(43,43),(40,40),(37,37),(34,34)])]),
    ]
    return wrap(cases, control_effect=0x10)
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
