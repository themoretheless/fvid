"""Owned partitioned MBAFF SI/intra/PCM with independent availability/matrix oracle."""
import json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from generate_avc_data_partitions import packet,mux
from generate_avc_si_mbaff_fixtures import configuration,positions,pcm_value,reconstruct,slice_nal as combined_slice
from generate_avc_switching_mbaff_fixtures import source,slice_nal as seed_slice,write_chroma_dc_one
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-data-partitions-mbaff-si'
# Adapted owned SI entropy writer: category2 remains in b; all category3 is in r.
def slice_nal(addresses,fields,switching,frame,qs,coded,pcm,chroma_mode=None,dc_level=1,mode=1,offsets=(0,0),raster=False,interlaced=False,partitioned=False):
    height=len(fields)*16
    b=Writer();r=Writer() if partitioned else b;used=False
    b.ue(addresses[0] if raster else addresses[0]//2);b.ue(4);b.ue(0);b.u(frame,4)
    if not raster or interlaced:b.u(0)
    if frame==0:b.ue(0)
    b.u(frame*2,4)
    if frame==0:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0);b.se(qs-26);b.ue(mode)
    if mode!=1:b.se(offsets[0]//2);b.se(offsets[1]//2)
    if partitioned:
        assert frame>0
        sid=addresses[0]//2;b.ue(sid);r.ue(sid)
    counts=[None]*(32*height)
    chroma_counts=[[None]*(8*height) for _ in range(2)]
    for address in addresses:
        if not raster and address%2==0:b.u(int(fields[address//2]))
        if address in pcm:
            b.ue(26);r.align();used=True
            for component,side in [(0,16),(1,8),(2,8)]:
                for _ in range(side*side):r.u(pcm_value(address,component),8)
            for pos in positions(address//2,address%2,fields[address//2],0,raster):counts[pos]=16
            for component in range(2):
                for pos in positions(address//2,address%2,fields[address//2],component+1,raster):chroma_counts[component][pos]=16
            continue
        b.ue(0 if switching[address] else 1)
        for _ in range(16):b.u(1) # predicted intra4 DC mode
        b.ue(0);active=coded and switching[address]
        b.ue((0 if chroma_mode in ['ac','both'] else (1 if chroma_mode=='dc' else 2)) if active else 3)
        if active:b.se(0);used=True
        locations=positions(address//2,address%2,fields[address//2],0,raster)
        step=2 if fields[address//2] else 1
        for block in range(16):
            bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2
            pos=locations[by*4*16+bx*4];y,x=divmod(pos,32)
            if active:
                neighbours=[counts[yy*32+xx] for xx,yy in [(x-1,y),(x,y-step)] if 0<=xx<32 and 0<=yy<height and counts[yy*32+xx] is not None]
                nc=(sum(neighbours)+len(neighbours)//2)//len(neighbours) if neighbours else 0
                # Table9-5, TotalCoeff1/TrailingOnes1; PCM contributes count16.
                token,length=(1,2) if nc<2 else ((2,2) if nc<4 else ((14,4) if nc<8 else (1,6)))
                r.u(token,length);r.u((block+address)%2);r.u(1)
            for yy in range(4):
                for xx in range(4):counts[locations[(by*4+yy)*16+bx*4+xx]]=int(active)
        if active and chroma_mode:
            for component in range(2):
                if chroma_mode in ['dc','both']:write_chroma_dc_one(r,(component+address)%2,dc_level)
                else:r.u(1,2)
        for component in range(2):
            locations=positions(address//2,address%2,fields[address//2],component+1,raster)
            for block in range(4):
                bx=block%2;by=block//2;pos=locations[by*4*8+bx*4];y,x=divmod(pos,16)
                ac=active and chroma_mode in ['ac','both']
                if ac:
                    counts_c=chroma_counts[component]
                    neighbours=[counts_c[yy*16+xx] for xx,yy in [(x-1,y),(x,y-step)] if 0<=xx<16 and 0<=yy<height//2 and counts_c[yy*16+xx] is not None]
                    nc=(sum(neighbours)+len(neighbours)//2)//len(neighbours) if neighbours else 0
                    token,length=(1,2) if nc<2 else ((2,2) if nc<4 else ((14,4) if nc<8 else (1,6)))
                    r.u(token,length);r.u((block+component+address)%2);r.u(1)
                for yy in range(4):
                    for xx in range(4):chroma_counts[component][locations[(by*4+yy)*8+bx*4+xx]]=int(ac)
    if not partitioned:return b.nal(0x65 if frame==0 else 0x41)
    return [b.nal(0x42)]+([r.nal(0x43)] if used else [])


def generate():
    files={};cases=[]
    patterns={'all':[True]*4,'top':[True,False,True,False],'bottom':[False,True,False,True],'left':[True,True,False,False],'right':[False,False,True,True],'pcm-top':[False,True,False,True],'pcm-bottom':[True,False,False,True]}
    for constrained in [False,True]:files[f'constrained{int(constrained)}.avcc']=configuration(constrained)
    for fields in [(False,False),(True,True),(False,True),(True,False)]:
        topology=f'{int(fields[0])}{int(fields[1])}';seedname='seed-'+topology+'.packet'
        seed=packet([seed_slice(pair,fields[pair],0,'primary',26,False,source(),'none') for pair in [0,1]]);files[seedname]=seed
        for pattern,switching in patterns.items():
         for constrained in [False,True]:
          for coded in [False,True]:
           for chroma_mode in [None,'both']:
            for split,aso in [(False,False),(True,False),(True,True)]:
                pcm={0} if pattern=='pcm-top' else ({1} if pattern=='pcm-bottom' else set())
                groups=[[0,1],[2,3]] if split else [[0,1,2,3]]
                name=f'{topology}-{pattern}-constrained{int(constrained)}-signed{int(coded)}-chroma{int(chroma_mode is not None)}-split{int(split)}-aso{int(aso)}'
                config_name=f'constrained{int(constrained)}.avcc';frames=[(0,True,seed)];records=[dict(packet=seedname,control=seedname,presentation=0,emits=True,qs=26)];gold=bytearray(sum(source(),[]))
                for frame,qs in enumerate([0,26,51],1):
                    order=groups[::-1] if aso else groups
                    parts=[slice_nal(group,fields,switching,frame,qs,coded,pcm,chroma_mode,partitioned=True) for group in order]
                    actual=packet([n for category in [2,3] for group in parts for n in group if n[0]&31==category] if aso else [n for group in parts for n in group])
                    control=packet([combined_slice(group,fields,switching,frame,qs,coded,pcm,chroma_mode) for group in order])
                    stem=f'{name}-{frame}';files[stem+'.packet']=actual;files[stem+'-control.packet']=control
                    frames.append((frame,False,actual));records.append(dict(packet=stem+'.packet',control=stem+'-control.packet',presentation=frame,emits=True,qs=qs))
                    gold.extend(reconstruct(fields,switching,qs,coded,constrained,groups,pcm,chroma_mode))
                files[name+'.mp4']=mux(files[config_name],frames,32,32,25);files[name+'.yuv']=bytes(gold)
                cases.append(dict(name=name,configuration=config_name,fields=fields,switching=switching,pcm=sorted(pcm),constrained=constrained,coded=coded,chroma=chroma_mode,split=split,aso=aso,groups=groups,file=name+'.mp4',oracle=name+'.yuv',frames=records))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned MBAFF SI/ordinary intra/PCM category2/3, scalar per-sample availability and switching matrix oracle; no private content, commands or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
