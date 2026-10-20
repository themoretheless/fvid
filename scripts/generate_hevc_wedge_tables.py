"""Own I.6.6 indexed wedge references, closed-form line rounding, offline."""
import json
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/hevc-wedge'
def generate():
    files={};cases=[]
    for size in [4,8,16]:
        shift=int(size<16);domain=size<<shift;patterns=[];seen=set();start=0
        def insert(p):
            p=bytes(p);inverse=bytes(1-v for v in p)
            if len(set(p))>1 and p not in seen:
                seen.update([p,inverse]);patterns.append(p)
        for orientation in range(6):
            end=len(patterns)
            if orientation in [0,4]:
                for m in range(0,domain,2 if size==16 else 1):
                    for n in range(0,domain,2 if size==16 and orientation==0 else 1):
                        a=(m,0);b=(0,n)if orientation==0 else(n,domain-1)
                        steep=abs(b[1]-a[1])>abs(b[0]-a[0]);end_y=b[1]
                        if steep:a=a[::-1];b=b[::-1]
                        if a[0]>b[0]:a,b=b,a
                        dx=b[0]-a[0];dy=abs(b[1]-a[1]);p=[0]*(size*size)
                        for x in range(a[0],b[0]+1):
                            step=(2*(x-a[0])*dy+dx)//(2*dx)if dx else 0
                            y=a[1]+(step if b[1]>a[1]else -step)
                            px,py=(y>>shift,x>>shift)if steep else(x>>shift,y>>shift)
                            p[py*size+px]=1
                        for y in range((end_y>>shift)+1):
                            row=p[y*size:(y+1)*size];first=row.index(1)if 1 in row else size
                            p[y*size:y*size+first]=[1]*first
                        insert(p)
            else:
                for index in range(start,end):
                    source=patterns[index]
                    insert([1-source[(size-1-x)*size+y]for y in range(size)for x in range(size)])
            start=end
        name=f'{size}.bin';files[name]=b''.join(patterns);cases.append(dict(size=size,count=len(patterns),file=name))
        if size==16:
            files['32.bin']=bytes(p[(y//2)*16+x//2]for p in patterns for y in range(32)for x in range(32))
            cases.append(dict(size=32,count=len(patterns),file='32.bin'))
    files['manifest.json']=(json.dumps(dict(provenance='Authored mathematical I.6.6 component masks; not video playback or independent interoperability proof.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
