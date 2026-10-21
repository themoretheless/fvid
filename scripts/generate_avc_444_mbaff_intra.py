"""Owned shared 4:4:4 MBAFF I16/I4/I8 and PCM; three independent physical-plane oracles."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_fields import configuration
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_mono_mbaff_intra import picture as mono_picture
from generate_avc_pcm_formats import Writer,mux
from avc_cabac_444_writer import Cabac444Writer
from generate_avc_separate_mbaff_8x8_deblock import reconstruct_i8x8_mb_exact,scatter_mbaff_plane,deblock_mbaff_plane
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-mbaff-intra'
def picture(depth,cabac,topology,index,kind="i16-dc",mode=1,witness=None):
    nals=[]
    pcmplanes=[[(80+20*p+index*7+2*((x//4+y//4)%2))*(1<<(depth-8))for y in range(64)for x in range(32)]for p in range(3)]
    for pair,field in enumerate(MODES[topology]):
        b=Writer();b.ue(pair);b.ue(2);b.ue(0);b.u(index,4);b.u(0)
        if index==0:b.ue(0)
        b.u(index*2,4)
        if index==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0);b.ue(mode)
        if mode!=1:b.se(6 if kind=="pcm" else 0);b.se(6 if kind=="pcm" else 0)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=Cabac444Writer(-1,26);c.decision(70,int(field))
        for parity in range(2):
            if kind=='pcm':
                if cabac:
                    c.decision(3+int(parity==1 and not field),1);b.bits.extend(c.finish());b.align()
                else:
                    if parity==0:b.u(int(field))
                    b.ue(25);b.align()
                for p in range(3):
                    for row in range(16):
                        y=pair//2*32+(row*2+parity if field else row+parity*16)
                        for col in range(16):b.u(pcmplanes[p][y*32+pair%2*16+col],depth)
                if cabac:c.low=0;c.range=510;c.width=9
                continue
            i16=kind=='i16-dc';eight=kind=='i8-dc'
            if cabac:
                c.decision(3+int(i16 and parity==1 and not field),int(i16))
                if i16:
                    c.range-=2;c.renormalize()
                    for ctx,v in [(6,0),(7,0),(9,1),(10,0),(60,0)]:c.decision(ctx,v)
                else:
                    # Shared transform_size_8x8_flag; top member may contribute.
                    c.decision(399+int(eight and parity==1 and not field),int(eight))
                    for _ in range(4 if eight else 16):c.decision(68,1)
                    for _ in range(4):c.decision(73,1)
                    c.decision(60,0)
            else:
                if parity==0:b.u(int(field))
                b.ue(3 if i16 else 0)
                if not i16:
                    b.u(int(eight))
                    for _ in range(4 if eight else 16):b.u(1)
                    b.ue(0)
                b.se(0)
            for p in range(3):
                negative=(pair+parity+index+p)%2==0
                if i16:
                    if cabac:
                        cbf=[85,460,472][p];sig=([277,776,820]if field else[105,484,528])[p]
                        last=([338,864,908]if field else[166,572,616])[p];absolute=[227,952,982][p]
                        for ctx,v in [(cbf+3,1),(sig,1),(last,1),(absolute+1,1),(absolute+5,1),(absolute+5,1),(absolute+5,0)]:c.decision(ctx,v)
                        c.bypass(int(negative))
                    else:b.u(5,6);b.u(1,6 if negative else 5);b.u(1)
                elif eight:
                    sign=(pair+parity+index+p)%2
                    if cabac:
                        cbf=[1012,1016,1020][p];sig=([436,675,733]if field else[402,660,718])[p]
                        last=([451,699,757]if field else[417,690,748])[p];absolute=[426,708,766][p]
                        for _ in range(4):
                            for ctx,v in [(cbf+3,1),(sig,1),(last,1),(absolute+1,1),(absolute+5,1),(absolute+5,1),(absolute+5,0)]:c.decision(ctx,v)
                            c.bypass(sign)
                    else:
                        for sub in range(16):
                            if sub%4==0:b.u(5,6);b.u(1,6 if sign else 5);b.u(1)
                            else:b.u(1)
                else:
                    for block in range(16):
                        sign=(pair+parity+index+p+block)%2
                        if cabac:
                            cbf=[93,468,480][p];sig=([306,805,849]if field else[134,513,557])[p]
                            last=([367,893,937]if field else[195,601,645])[p];absolute=[247,972,1002][p]
                            c.decision(cbf+3,1)
                            if kind=='i4-ac':c.decision(sig,0);c.decision(sig+1,1);c.decision(last+1,1)
                            else:c.decision(sig,1);c.decision(last,1)
                            c.decision(absolute+1,0);c.bypass(sign)
                        else:
                            b.u(1,2);b.u(sign)
                            if kind=='i4-ac':b.u(3,3)
                            else:b.u(1)
        if cabac:b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index==0 else 0x41,trailing=not cabac))
    packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
    if kind=='pcm':planes=pcmplanes
    elif kind=='i8-dc':
        planes=[]
        for p in range(3):
            pairs=[]
            for pair,field in enumerate(MODES[topology]):
                mbs=[];top=None
                for parity in range(2):
                    sign=1 if (pair+parity+index+p)%2==0 else -1
                    mb=reconstruct_i8x8_mb_exact(depth,sign,16,top_row=top if parity and not field else None)
                    mbs.append(mb);top=mb[15*16:16*16]
                pairs.append(mbs)
            planes.append(scatter_mbaff_plane(pairs,topology))
    else:planes=[mono_picture(depth,cabac,topology,False,kind,index,colour_plane=p)[1]for p in range(3)]
    unfiltered=planes
    planes=[deblock_mbaff_plane(plane,depth,topology,mode,True,transform8=kind=='i8-dc',qp=(12 if p==0 else 24) if kind=='pcm' else 26)for p,plane in enumerate(planes)]
    if witness is not None and mode!=1:
        key=f"{kind}-f{mode}"
        witness[key]=witness.get(key,0)+sum(a!=b for raw,filtered in zip(unfiltered,planes)for a,b in zip(raw,filtered))
    return packet,sum(planes,[])
def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness={}
    for depth,cabac,topology,kind,mode in itertools.product([8,10,14],[False,True],MODES,["i16-dc","i4-dc","i4-ac","i8-dc","pcm"],[0,1,2]):
        name=f'shared-y{depth}-e{int(cabac)}-{topology}-{kind}-f{mode}'
        cfg=configuration(depth,depth,cabac,[12,12]if kind=="pcm" else[0,0],mbaff=True);packets=[];words=[]
        for i in range(2):
            packet,plane=picture(depth,cabac,topology,i,kind,mode,witness);packets.append(packet);words+=plane
            (OUT/f'{name}-{i}.packet').write_bytes(packet)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<12288H',*words))]:
            (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,depth=depth,cabac=cabac,topology=topology,kind=kind,mode=mode))
    assert len(cases)==360
    assert len(witness)==10 and all(v>0 for v in witness.values())
    (OUT/'witnesses.json').write_text(json.dumps(witness,indent=2)+'\n')
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
