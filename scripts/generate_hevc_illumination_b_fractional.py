"""Own fractional mixed B-list IC, preserving internal precision in its oracle."""
import json,struct
from generate_hevc_illumination_b import movie,pattern,MODES
from generate_hevc_illumination import parameters,internal_plane
from generate_hevc_cqt import OUT

def oracle(size,enabled,mode,gain,mvs,disabled_merge_zero=False):
    planes=[];changed=0;precision_changed=0
    for c in range(3):
        side=size if c==0 else size//2;cu=32 if c==0 else 16
        filtered=[internal_plane(size,c,*mv)for mv in mvs];out=[0]*(side*side)
        for y in range(0,side,cu):
            for x in range(0,side,cu):
                first=x==0 and y==0;refs=(0,0)if first else MODES[mode];params=[]
                active=enabled and (not disabled_merge_zero or (x,y)==(cu,0) or (x==0 and y==cu and mode!='temporal'))
                for list,index in enumerate(refs):
                    cross=index==(1 if list==0 else 2)
                    cur=[];ref=[];mv=mvs[list]if cross else(0,0)
                    shift=2 if c==0 else 3;dx,dy=[(v+(1<<(shift-1)))>>shift for v in mv]
                    def source(xx,yy):return pattern(min(side-1,max(0,xx+dx)),min(side-1,max(0,yy+dy)),c,False,0,gain)
                    if y:
                        cur.extend(out[(y-1)*side+x+i]for i in range(cu));ref.extend(source(x+i,y-1)for i in range(cu))
                    if x:
                        cur.extend(out[(y+i)*side+x-1]for i in range(cu));ref.extend(source(x-1,y+i)for i in range(cu))
                    params.append(parameters(cur,ref,c)if cross and cur else(32,0))
                for yy in range(y,y+cu):
                    for xx in range(x,x+cu):
                        values=[];raw=[]
                        for list,index in enumerate(refs):
                            if index is None:continue
                            cross=index==(1 if list==0 else 2)
                            internal=filtered[list][yy*side+xx]if cross else pattern(xx,yy,c,True,97 if list==1 else 0,gain)*64
                            raw.append(internal);value=min(255,max(0,(internal+32)>>6))
                            weight,offset=params[list]
                            values.append(min(255,max(0,min(255,(value*weight)>>5)+offset))if cross else value)
                        base=min(255,max(0,(sum(raw)+(32 if len(raw)==1 else 64))>>(6 if len(raw)==1 else 7)))
                        if len(raw)==2:
                            prematurely_rounded=(sum(min(255,max(0,(v+32)>>6))for v in raw)+1)//2
                            precision_changed+=prematurely_rounded!=base
                        value=(values[0]if len(values)==1 else(sum(values)+1)//2)if active else base
                        out[yy*side+xx]=value;changed+=value!=base
        planes.extend(out)
    return planes,changed,precision_changed

def generate():
    files={};cases=[];golden=bytearray()
    for size in [64,128]:
        for direct in [False,True]:
            for enabled in [False,True]:
                gain=(2,1,-40)
                for mode in MODES:
                    for mvs in [((1,3),(-3,-1)),((-7,5),(5,-7)),((2,-3),(-1,6))]:
                        suffix='-'.join(str(v)for mv in mvs for v in mv)
                        name=f'illumination-b-fractional-{size}-{int(direct)}-{int(enabled)}-{mode}-{suffix}.mp4'
                        files[name]=movie(size,direct,enabled,mode,gain,mvs)
                        values,changed,precision=oracle(size,enabled,mode,gain,mvs);at=len(golden)//2
                        golden.extend(struct.pack('<'+str(len(values))+'H',*values))
                        cases.append(dict(file=name,size=size,direct=direct,enabled=enabled,target_luma=gain,mode=mode,references=MODES[mode],motion=mvs,oracle_offset=at,changed_samples=changed,precision_witnesses=precision))
    files['illumination-b-fractional.y16']=bytes(golden)
    files['illumination-b-fractional.json']=(json.dumps(dict(provenance='Owned explicit two-list fractional B MV syntax, independent internal-precision FIR, per-list IC regression and combination oracle. No private media or external codec.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
