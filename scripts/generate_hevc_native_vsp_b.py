"""Owned B-slice VSP with independent saved subblock luma oracle, offline."""
import json,struct
import generate_hevc_texture_iv_b as b
import generate_hevc_refinement_b as r
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_native_vsp import oracle
from generate_hevc_cqt import reference,OUT,banks,context,merge_index_bins
from hevc_fixture_mp4 import mux

def residual_target(residual,ac=False):
    encoder,c=banks(2)
    encoder.decision(c['split0'],False);encoder.decision(c['skip0'],False)
    encoder.decision(c['pred'],False);encoder.decision(c['part0'],True)
    encoder.decision(c['merge'],True);merge_index_bins(encoder,c,0,5)
    levels = residual if isinstance(residual,tuple) else (residual,0,0)
    split=context(encoder,224);chroma=context(encoder,149)
    luma_cbf=context(encoder,111)
    contexts=[(context(encoder,x),context(encoder,x),context(encoder,g),context(encoder,107),context(encoder,170))for x,g in [(125,196),(108,208)]]
    encoder.decision(split,False)
    encoder.decision(chroma,levels[1]!=0);encoder.decision(chroma,levels[2]!=0)
    if levels[1] or levels[2]:encoder.decision(luma_cbf,levels[0]!=0)
    for component,level in enumerate(levels):
        if not level:continue
        last_x,last_y,greater,greater2,significant=contexts[int(component!=0)]
        encoder.decision(last_x,False)
        if ac:encoder.decision(last_y,True)
        encoder.decision(last_y,False)
        if ac:encoder.decision(significant,False)
        encoder.decision(greater,True);encoder.decision(greater2,True)
        encoder.bypass(level<0)
        for bit in '11111111011011':encoder.bypass(int(bit))
    return encoder.finish()

def colored_pcm(target,bias=0):
    encoder,c=banks(0);encoder.decision(c['split0'],False)
    prefix=encoder.finish()
    pixels=bytes((3*x+5*y+17+bias)%256 for y in range(32)for x in range(32))
    for c in [1,2]:
        pixels+=bytes((13*x+7*y+29+83*(c-1)+([71,113][c-1]if target else 0)+bias)%256 for y in range(16)for x in range(16))
    encoder.low=0;encoder.range=510;encoder.width=9
    return prefix+pixels+encoder.finish()

def movie(scale,weighted,l1_only=False,residual=None,colored=False,ac=False):
    config_movie=control_movie(True,camera=(scale,0,0),vsp=True)
    config=config_movie[config_movie.index(b'hvcC')+4:];at=23;params=[]
    for _ in range(config[22]):
        count=int.from_bytes(config[at+1:at+3],'big');at+=3
        for _ in range(count):
            n=int.from_bytes(config[at:at+2],'big');at+=2
            nal=config[at:at+n];at+=n
            if weighted and ((int.from_bytes(nal[:2],'big')>>9)&63)==34 and ((int.from_bytes(nal[:2],'big')>>3)&63)==5:
                nal=r.parameter_ids(r.header_pps(5,0,True),0,[2,2])
            params.append(nal)
    past=colored_pcm(False) if colored else b.pcm()
    future=colored_pcm(False,97) if colored else b.pcm(97)
    target_past=colored_pcm(True) if colored else past
    target_future=colored_pcm(True,97) if colored else future
    depth,_=reference(32,5,False);units=[]
    for poc in [0,2,1]:
        units.append([
            b.header(0,poc,future if poc==2 else past,2,negative=[2]if poc==2 else[1]if poc==1 else[],positive=[1]if poc==1 else[]),
            r.header(3,poc,depth,True),
            r.header(5,poc,(b.target(0) if residual is None else residual_target(residual,ac))if poc==1 else target_future if poc==2 else target_past,True,weighted,list_entries=([ [0,2,0], None ] if l1_only and poc==1 else None))])
    stream=b''.join(b'\0\0\0\1'+n for n in params+[n for au in units for n in au])
    return mux(stream,8,32,32,chroma_format=1,access_units=units,multilayer=True,presentation_order=[0,2,1])

def generate(l1_only=False):
    files={};cases=[];golden=bytearray()
    for weighted in [False,True]:
        for scale in [4,-4,128,-128,0]:
            name=f'native-vsp-b{"-l1" if l1_only else ""}-{int(weighted)}-{scale}.mp4';files[name]=movie(scale,weighted,l1_only)
            pixels,motion,_,_=oracle(32,scale,0,clip_output=False)
            # Default first match: L0[1], uni weight 2/2 and offset -3.
            # L1-only removes the source view from L0: L1[2], weight 0 and offset 9.
            # Unweighted cases preserve actual interpolation; chroma stays neutral.
            values=[(9 if l1_only else min(255,max(0,v-3))) if weighted else min(255,max(0,v)) for v in pixels[:1024]]+[128]*512
            at=len(golden)//2;golden.extend(struct.pack('<1536H',*values))
            cases.append(dict(file=name,weighted=weighted,scale=scale,oracle_offset=at,motion=motion))
    prefix='native-vsp-b-l1' if l1_only else 'native-vsp-b'
    files[prefix+'.y16']=bytes(golden)
    files[prefix+'.json']=(json.dumps(dict(provenance='Owned PCM/CABAC B slices, independent VSP depth and interpolation oracle. No external codecs or private data.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for l1_only in [False,True]:
        for name,data in generate(l1_only).items():(OUT/name).write_bytes(data)
