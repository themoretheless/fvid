"""Literal nearest-DLT and depth DC block references, offline."""
import json,struct
from pathlib import Path
import generate_hevc_depth_tables as t
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/hevc-depth-reconstruction'
def generate():
    source=t.generate();m=json.loads(source['manifest.json']);files={};cases=[]
    for c in m['cases']:
        if c['error'] or c['expected']is None:continue
        depth=c['depth'];limit=(1<<depth)-1
        for layer,entries in zip(c['ids'],c['expected']):
            name=c['name']+f'-layer{layer}'
            # Exhaustive scalar search deliberately differs from native binary search.
            inverse=None if entries==[] else [sample if entries is None else min(range(len(entries)),key=lambda i:(abs(entries[i]-sample),-i))for sample in range(limit+1)]
            lookup=list(range(limit+1))if entries is None else entries+[0]*(limit+1-len(entries))
            files[name+'-lookup.bin']=struct.pack('<'+'H'*len(lookup),*lookup)
            if inverse is not None:files[name+'-inverse.bin']=struct.pack('<'+'H'*len(inverse),*inverse)
            predictions=[];blocks=[]
            if inverse is not None:
                for sample in [0,1,3,5,limit//2,limit-1,limit]:
                    for offset in [-limit-1,-2,-1,0,1,2,limit+1,-(1<<63),(1<<63)-1]:
                        index=max(0,min(limit,inverse[sample]+offset))
                        predictions.append(dict(sample=sample,offset=offset,expected=str(sample+offset if entries is None else lookup[index])))
                block=[(13+7*i)%(limit+1)for i in range(16)];block[5]=limit;block[10]=0
                predictor=(sum(block[i]for i in [0,3,12,15])+2)//4
                for offset in [-limit-1,-2,-1,0,1,2,limit+1,-(1<<63),(1<<63)-1]:
                    dc=offset if entries is None else lookup[max(0,min(limit,inverse[predictor]+offset))]-predictor
                    blocks.append(dict(input=block,offset=offset,expected=[max(0,min(limit,value+dc))for value in block]))
            cases.append(dict(name=name,source=c['name'],bits=c['bits'],depth=depth,ids=c['ids'],layer=layer,inverse=inverse is not None,predictions=predictions,blocks=blocks))
    files['manifest.json']=(json.dumps(dict(provenance='Own literal/scalar exhaustive nearest-index reference, upper tie selection, zero-padded I-55 domain, four-corner DC assignment. No full 3D playback claim.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
