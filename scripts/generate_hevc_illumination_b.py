"""Owned coded B-list IC with independent per-list CU boundary/sample oracle."""
import json,struct
import generate_hevc_texture_iv_b as b
import generate_hevc_refinement_b as r
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_illumination import parameters
from generate_hevc_cqt import reference,OUT,banks,context,merge_index_bins,terminal_zero
from hevc_fixture_mp4 import mux
MODES={'both':(1,2),'past-source':(0,2),'source-future':(1,0),'l0':(1,None),'l1':(None,2),'temporal':(0,0)}

def pattern(x,y,c,target,bias,gain):
    if c==0:
        value=(3*x+5*y+17+bias)%256
        num,den,offset=gain
        return min(255,max(0,value*num//den+offset))if target else value
    return (13*x+7*y+29+83*(c-1)+([71,113][c-1]if target else 0)+bias)%256

def pcm(size,target,bias,gain):
    encoder,c=banks(0);chunks=[];cols=size//32
    for cy in range(cols):
        for cx in range(cols):
            encoder.decision(c['split0'],False);chunks.append(encoder.finish())
            for component in range(3):
                side=32 if component==0 else 16
                chunks.append(bytes(pattern(cx*side+x,cy*side+y,component,target,bias,gain)for y in range(side)for x in range(side)))
            encoder.low=0;encoder.range=510;encoder.width=9
            if cy*cols+cx+1<cols*cols:terminal_zero(encoder)
    chunks.append(encoder.finish());return b''.join(chunks)

def target(size,enabled,mode,mvs=None,disabled_merge_zero=False,residual_prediction=None,weighted=False,references=None,temporal_motion=False,first_merge_index=2,additional_mvs=None):
    encoder,c=banks(2);ic=context(encoder,154);ref=[context(encoder,153),context(encoder,153)];greater=context(encoder,198)
    cols=size//32;skips=[];refs=MODES[mode]if references is None else references
    rp=[context(encoder,v)for v in [162,153,162]]if residual_prediction is not None else None
    residual_weights=[]
    for cy in range(cols):
        for cx in range(cols):
            explicit=(cx,cy)==(1,0)or additional_mvs is not None and(cx,cy)==(0,1)
            encoder.decision(c['split0'],False)
            count=int(cx>0 and skips[-1])+int(cy>0 and skips[(cy-1)*cols+cx])
            encoder.decision(c[f'skip{count}'],not explicit);skips.append(not explicit)
            if explicit:
                encoder.decision(c['pred'],False);encoder.decision(c['part0'],True);encoder.decision(c['merge'],False)
                bi=all(v is not None for v in refs);encoder.decision(c['ip0'],bi)
                if not bi:encoder.decision(c['ip4'],refs[1] is not None)
                for which,index in enumerate(refs):
                    if index is None:continue
                    encoder.decision(ref[0],index>0)
                    if index>0:encoder.decision(ref[1],index>1)
                    second=additional_mvs is not None and(cx,cy)==(0,1)
                    wanted=(additional_mvs if second else mvs)
                    vector=wanted[which]if wanted is not None and (temporal_motion or index==(1 if which==0 else 2))else(0,0)
                    if second:vector=tuple(v-p for v,p in zip(vector,mvs[which]))
                    for value in vector:encoder.decision(c['mvd'],value!=0)
                    for value in vector:
                        if value:encoder.decision(greater,abs(value)>1)
                    for value in vector:
                        if abs(value)>1:
                            remaining=abs(value)-2;k=1
                            while remaining>=1<<k:encoder.bypass(True);remaining-=1<<k;k+=1
                            encoder.bypass(False)
                            for shift in reversed(range(k)):encoder.bypass((remaining>>shift)&1)
                        if value:encoder.bypass(value<0)
                    encoder.decision(c['mvp'],False)
            else:merge_index_bins(encoder,c,first_merge_index if cx==0 and cy==0 else 1 if cx==0 and cy==1 and (mode!='temporal' or temporal_motion) else 0,5)
            # Explicit temporal-only motion infers IC off; merge flags can be present even for temporal references.
            index=first_merge_index if cx==0 and cy==0 else 1 if cx==0 and cy==1 and (mode!='temporal' or temporal_motion) else 0
            weight=residual_prediction[cy*cols+cx]if isinstance(residual_prediction,(list,tuple))else residual_prediction
            if rp is not None:
                encoder.decision(rp[int(bool(cx and residual_weights[-1]))],weight!=0)
                if weight:encoder.decision(rp[2],weight==2)
                residual_weights.append(weight)
            if not weighted and not weight and ((not explicit and (index!=0 or not disabled_merge_zero)) or (explicit and any(index==(1 if direction==0 else 2)for direction,index in enumerate(refs)))):encoder.decision(ic,enabled)
            if explicit:encoder.decision(c['root'],False)
            if cy*cols+cx+1<cols*cols:terminal_zero(encoder)
    return encoder.finish()

def movie(size,direct,enabled,mode,gain,mvs=None,disabled_merge_zero=False,residual_prediction=None,weighted=False,references=None,temporal_motion=False,inter_view=True,source_data=None,source_kind=2,additional_mvs=None):
    raw=control_movie(False,size=size,direct=direct,colored=True,residual_prediction=residual_prediction,inter_view=inter_view)
    config=raw[raw.index(b'hvcC')+4:];at=23;params=[]
    for _ in range(config[22]):
        count=int.from_bytes(config[at+1:at+3],'big');at+=3
        for _ in range(count):
            length=int.from_bytes(config[at:at+2],'big');at+=2;nal=config[at:at+length];at+=length
            if weighted and ((int.from_bytes(nal[:2],'big')>>9)&63)==34 and ((int.from_bytes(nal[:2],'big')>>3)&63)==5:
                from generate_hevc_layered_slices import header_pps
                from generate_hevc_layered_reconstruction import parameter_ids
                nal=parameter_ids(header_pps(5,0,True,constrained=direct),0,[2,2])
            params.append(nal)
    past=pcm(size,False,0,gain);future=pcm(size,False,97,gain)
    own_past=pcm(size,True,0,gain);own_future=pcm(size,True,97,gain)
    depth,_=reference(size,5,False);units=[]
    for poc in [0,2,1]:
        units.append([b.header(0,poc,future if poc==2 else source_data if poc==1 and source_data is not None else past,source_kind if poc==1 else 2,negative=[2]if poc==2 else[1]if poc==1 else[],positive=[1]if poc==1 else[]),
            r.header(3,poc,depth,False),
            r.header(5,poc,target(size,enabled,mode,mvs,disabled_merge_zero,residual_prediction,weighted,references,temporal_motion,2 if inter_view else 0,additional_mvs)if poc==1 else own_future if poc==2 else own_past,False,weighted=weighted,explicit=weighted,illumination=enabled,ic_disabled_merge_zero=disabled_merge_zero,merge_extra=int(inter_view))])
    stream=b''.join(b'\0\0\0\1'+n for n in params+[n for au in units for n in au])
    return mux(stream,8,size,size,chroma_format=1,access_units=units,multilayer=True,presentation_order=[0,2,1])

def oracle(size,enabled,mode,gain,disabled_merge_zero=False):
    planes=[];changed=0
    for c in range(3):
        side=size if c==0 else size//2;cu=32 if c==0 else 16;out=[0]*(side*side)
        for y in range(0,side,cu):
            for x in range(0,side,cu):
                first=x==0 and y==0
                refs=(0,0)if first else MODES[mode]
                cur=[];ref=[]
                def source(xx,yy):return pattern(xx,yy,c,False,0,gain)
                if y:
                    cur.extend(out[(y-1)*side+x+i]for i in range(cu));ref.extend(source(x+i,y-1)for i in range(cu))
                if x:
                    cur.extend(out[(y+i)*side+x-1]for i in range(cu));ref.extend(source(x-1,y+i)for i in range(cu))
                active=enabled and (not disabled_merge_zero or (x,y)==(cu,0) or (x==0 and y==cu and mode!='temporal'))
                weight,offset=parameters(cur,ref,c)if active and cur else(32,0)
                for yy in range(y,y+cu):
                    for xx in range(x,x+cu):
                        values=[];ordinary=[]
                        for list,index in enumerate(refs):
                            if index is None:continue
                            cross=index==(1 if list==0 else 2)
                            value=source(xx,yy)if cross else pattern(xx,yy,c,True,97 if list==1 else 0,gain)
                            ordinary.append(value)
                            values.append(min(255,max(0,min(255,(value*weight)>>5)+offset))if active and cross else value)
                        value=values[0]if len(values)==1 else(sum(values)+1)//2
                        base=ordinary[0]if len(ordinary)==1 else(sum(ordinary)+1)//2
                        out[yy*side+xx]=value;changed+=value!=base
        planes.extend(out)
    return planes,changed

def generate():
    files={};cases=[];golden=bytearray()
    for size in [64,128]:
        for direct in [False,True]:
            for enabled in [False,True]:
                for gain in [(2,1,-40),(1,2,30)]:
                    for mode in MODES:
                        name=f'illumination-b-{size}-{int(direct)}-{int(enabled)}-'+ '-'.join(map(str,gain))+f'-{mode}.mp4'
                        files[name]=movie(size,direct,enabled,mode,gain)
                        values,changed=oracle(size,enabled,mode,gain);at=len(golden)//2
                        golden.extend(struct.pack('<'+str(len(values))+'H',*values))
                        cases.append(dict(file=name,size=size,direct=direct,enabled=enabled,target_luma=gain,mode=mode,references=MODES[mode],oracle_offset=at,changed_samples=changed))
    files['illumination-b.y16']=bytes(golden)
    files['illumination-b.json']=(json.dumps(dict(provenance='Owned PCM/CABAC mixed-view B pictures, independent per-list illumination and boundary oracle; no private data or external codecs.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
