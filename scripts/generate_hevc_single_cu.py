"""Own complete 64x64 skip-single CTU entropy payloads; parameters supplied by tests."""
from generate_hevc_depth_dc import Encoder
from generate_hevc_wedge_tables import OUT
def generate():
    files={}
    for size in [64,128]:
        for mode in range(4):
            b=Encoder(0,26)
            for value in [185,137,139]:
                pre=max(1,min(126,(((value>>4)*5-45)*26>>4)+(value&15)*8-16))
                b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
            count=(size//64)**2
            for ctu in range(count):
                b.decision(5,False)
                b.decision(3,True)
                b.decision(4,mode!=0)
                if mode:b.bypass(mode>1)
                if mode>1:b.bypass(mode==3)
                if ctu+1<count:
                    b.range-=2
                    while b.range<256:b.low*=2;b.range*=2;b.width+=1
            suffix=''if size==64 else '-128'
            files[f'single-cu-{mode}{suffix}.cabac']=b.finish()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
