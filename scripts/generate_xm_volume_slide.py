"""Owned XM volume slides versus literal tick-by-tick Cxx scores; offline."""
import json,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime,cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-volume-slide'
def generate(cases_override=None, control_effect=0xc, control_tick=None):
    source=lifetime();base=source['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0]
    body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=bytearray(base[:at])
    struct.pack_into('<HH',header,76,4,125)
    def module(rows,speed,channels=1):
        h=bytearray(header);struct.pack_into('<H',h,76,speed);struct.pack_into('<H',h,68,channels)
        payload=b''.join(rows)
        return bytes(h)+struct.pack('<IBHH',9,0,len(rows),len(payload))+payload+body
    # Each tuple is one row and an independently authored volume at each tick.
    cases=[
      ('down',[(cell(49,1,0x50,0xA,4),[64,60,56,52]),(cell(),[52]*4)]),
      ('up',[(cell(49,1,0x20,0xA,0x30),[16,19,22,25])]),
      ('both-nibbles-up-wins',[(cell(49,1,0x30,0xA,0x23),[32,34,36,38])]),
      ('clamp-high',[(cell(49,1,0x4c,0xA,0xf0),[60,64,64,64])]),
      ('clamp-low',[(cell(49,1,0x13,0xA,0x0f),[3,0,0,0])]),
      ('memory',[(cell(49,1,0x30,0xA,3),[32,29,26,23]),(cell(),[23]*4),(cell(effect=0xA),[23,20,17,14])]),
      ('empty-memory',[(cell(49,1,0x30,0xA),[32]*4)]),
      ('memory-note-trigger',[(cell(49,1,0x30,0xA,3),[32,29,26,23]),(cell(50,1,0x30,0xA),[32,29,26,23])]),
      ('speed-one-does-not-store',[(cell(49,1,0x30,0xA,4),[32,28,24,20]),(cell(effect=15,parameter=1),[20]),(cell(effect=0xA,parameter=2),[20]),(cell(effect=15,parameter=4),[20]*4),(cell(effect=0xA),[20,16,12,8])]),
      ('channel-local-memory',[(cell(49,1,0x30,0xA,4)+cell(50,1,0x20,0xA,3),[(32,16),(28,13),(24,10),(20,7)]),(cell(effect=0xA)+cell(effect=0xA),[(20,7),(16,4),(12,1),(8,0)])]),
    ]
    if cases_override is not None: cases=cases_override
    files={'video.mkv':source['video.mkv']};manifest=[]
    for name,rows in cases:
        actual=[c for c,_ in rows];flat=[];channels=len(actual[0])//5
        for c,levels in rows:
            for tick,level in enumerate(levels):
                levels_per_channel=[level] if channels==1 else level
                if control_tick is not None:
                    flat.append(control_tick(c,tick,levels_per_channel))
                else:
                    flat.append(b''.join(cell(c[5*ch] if tick==0 else 0,c[5*ch+1] if tick==0 else 0,effect=control_effect,parameter=v)for ch,v in enumerate(levels_per_channel)))
        end=cell(97,effect=15,parameter=4)+cell(97)*(channels-1)
        actual.append(end);flat.append(end)
        files[name+'.xm']=module(actual,4,channels);files[name+'-ticks.xm']=module(flat,1,channels)
        manifest.append(dict(name=name,file=name+'.xm',control=name+'-ticks.xm',volumes=[v for _,vs in rows for v in vs]))
    files['manifest.json']=(json.dumps(dict(cases=manifest,provenance='Owned signed16loop and authored literal tick-volume controls. No codec tools, network or private media.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
