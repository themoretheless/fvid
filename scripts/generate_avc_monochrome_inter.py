"""Owned monochrome progressive P skip/integer-motion witnesses."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-monochrome-inter'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for depth in [8,10,14]:
        for shift in [0,-4,4]:
            name=f'mono-p-{depth}-shift{shift}';config=configuration(0,depth,8,False)
            source=[(x*17+y*23+9)%(1<<depth)for y in range(16)for x in range(32)]
            frames=[]
            for index in range(2):
                b=Writer();b.ue(0);b.ue(2 if index==0 else 0);b.ue(0);b.u(index,4)
                if index==0:b.ue(0)
                b.u(index*2,4)
                if index==0:b.u(0);b.u(0)
                else:b.u(0);b.u(0);b.u(0) # reference override, list modification, marking
                b.se(0);b.ue(1)
                if index==0:
                    for mb in range(2):
                        b.ue(25);b.align()
                        for y in range(16):
                            for x in range(16):b.u(source[y*32+mb*16+x],depth)
                elif shift==0:b.ue(2)
                else:
                    for mb in range(2):
                        b.ue(0);b.ue(0);b.se(shift*4 if mb==0 else 0);b.se(0);b.ue(0)
                nal=b.nal(0x65 if index==0 else 0x41);packet=len(nal).to_bytes(4,'big')+nal
                (OUT/f'{name}-{index}.packet').write_bytes(packet);frames.append((index,index==0,packet))
            expected=source+[source[y*32+max(0,min(31,x+shift))]for y in range(16)for x in range(32)]
            (OUT/(name+'.avcc')).write_bytes(config);(OUT/(name+'.mp4')).write_bytes(mux(config,frames,32,16,25));(OUT/(name+'.y16')).write_bytes(struct.pack('<1024H',*expected));cases.append(dict(name=name,depth=depth,shift=shift))
    cases.extend(cabac_skips())
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
def cabac_skips():
    cases=[]
    for depth in [8,10,14]:
        name=f'mono-p-{depth}-cabac-skip';config=configuration(0,depth,8,False,True)
        source=[(x*17+y*23+9)%(1<<depth)for y in range(16)for x in range(32)]
        frames=[]
        for index in range(2):
            nals=[]
            for address in (range(2) if index==0 else [0]):
                b=Writer();b.ue(address);b.ue(2 if index==0 else 0);b.ue(0);b.u(index,4)
                if index==0:b.ue(0)
                b.u(index*2,4)
                if index==0:b.u(0);b.u(0)
                else:b.u(0);b.u(0);b.u(0);b.ue(0) # P CABAC init0
                b.se(0);b.ue(1)
                while len(b.bits)%8:b.u(1)
                c=CabacWriter(-1 if index==0 else 0,26)
                if index==0:
                    c.decision(3,1);b.bits.extend(c.finish());b.align()
                    for y in range(16):
                        for x in range(16):b.u(source[y*32+address*16+x],depth)
                    c.low=0;c.range=510;c.width=9;b.bits.extend(c.finish())
                else:
                    c.decision(11,1);c.range-=2;c.renormalize();c.decision(11,1);b.bits.extend(c.finish())
                nals.append(b.nal(0x65 if index==0 else 0x41,trailing=False))
            packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
            (OUT/f'{name}-{index}.packet').write_bytes(packet);frames.append((index,index==0,packet))
        (OUT/(name+'.avcc')).write_bytes(config);(OUT/(name+'.mp4')).write_bytes(mux(config,frames,32,16,25));(OUT/(name+'.y16')).write_bytes(struct.pack('<1024H',*(source*2)))
        cases.append(dict(name=name,depth=depth,shift=0,cabac=True))
    return cases
if __name__=='__main__':generate()
