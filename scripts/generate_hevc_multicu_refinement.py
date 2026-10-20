"""Owned variable-depth multi-CU refinement with a separate scalar oracle."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_wedge_tables import OUT
FILTERS=[(0,0,0,64,0,0,0,0),(-1,4,-10,58,17,-5,1,0),(-1,4,-11,40,40,-11,4,-1),(0,1,-5,17,58,-10,4,-1)]

def oracle(size,scale,offset):
    columns=size//32;vectors=[];blocks=[];pixels=[0]*(size*size);witnesses=0
    def maximum(x,y,base):
        dx=(base+2)>>2
        return max((7*min(size-1,max(0,x+dx+xx))+11*(y+yy)+37)%256 for yy in [0,31]for xx in [0,31])
    for cy in range(columns):
        for cx in range(columns):
            index=cy*columns+cx;x=cx*32;y=cy*32
            base=vectors[index-1]if cx else vectors[index-columns]if cy else 0
            depth=maximum(x,y,base);value=(scale*depth+offset*256+64)>>7
            naive=(scale*maximum(x,y,0)+offset*256+64)>>7
            witnesses+=value!=naive
            a1=vectors[index-1]if cx else None
            b1=vectors[index-columns]if cy else None
            b0=vectors[index-columns+1]if cy and cx+1<columns else None
            prefix=[]
            if a1 is not None:prefix.append(a1)
            if b1 is not None and b1!=a1:prefix.append(b1)
            if b0 is not None and b0!=b1:prefix.append(b0)
            # Annex I inserts DI after A1/B1/B0, pruning it against A1/B1.
            merge_index=prefix.index(value)if value in [a1,b1]else len(prefix)
            vectors.append(value);blocks.append(dict(origin=[x,y],base=base,depth=depth,vector=value,merge_index=merge_index))
            dx,phase=divmod(value,4)
            for yy in range(y,y+32):
                for xx in range(x,x+32):
                    filtered=sum(w*((3*min(size-1,max(0,xx+dx+i-3))+5*yy+17)%256)for i,w in enumerate(FILTERS[phase]))
                    pixels[yy*size+xx]=min(255,max(0,(filtered+32)>>6))
    return pixels,blocks,witnesses

def generate():
    files={};cases=[];golden=bytearray();witnesses=0
    for size in [64,128]:
        for direct in [False,True]:
            for scale,offset in [(128,0),(-128,0),(4,0),(-4,0),(16,-3),(-16,3),(1,0),(-1,0)]:
                name=f'multicu-refinement-{size}-{int(direct)}-{scale}-{offset}.mp4'
                pixels,blocks,count=oracle(size,scale,offset);witnesses+=count;at=len(golden)//2
                files[name]=control_movie(True,camera=(scale,offset,0),size=size,direct=direct,
                    merge_indices=[b['merge_index']for b in blocks])
                golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                cases.append(dict(file=name,size=size,direct=direct,scale=scale,offset=offset,blocks=blocks,oracle_offset=at))
    files['multicu-refinement.y16']=bytes(golden)
    files['multicu-refinement.json']=(json.dumps(dict(
        provenance='Owned PCM/CABAC/MP4. Independent left/top disparity propagation, shifted corner depth maximum, camera arithmetic and 8-tap luma oracle. No external codec or test-time generation.',
        base_propagation_witnesses=witnesses,cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
