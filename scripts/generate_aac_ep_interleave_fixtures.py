"""Independent occupied-cell matrix oracle for MPEG-4 EP recursive stages."""
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ep-interleave'
def stage(x,y,width):
    assert width>0
    total=len(x)+len(y)
    if not total:return []
    rows=[[None]*min(width,total-i)for i in range(0,total,width)]
    for i,value in enumerate(x):rows[i//width][i%width]=value
    it=iter(y)
    for col in range(min(width,total)):
        for row in rows:
            if col<len(row)and row[col]is None:row[col]=next(it)
    assert next(it,None)is None
    return [row[col]for col in range(min(width,total))for row in rows if col<len(row)]
def generate():
    rows=[];blob=bytearray()
    totals=list(range(65))+[127,128,129,255,256,257,511,512,513]
    widths=list(range(1,33))+[63,64,127,128,255,256,257,1024]
    for total in totals:
        for width in widths:
            cuts=sorted({0,total,total//2,*[max(0,min(total,n))for n in [1,width-1,width,width+1,total-1]]})
            for lx in cuts:
                x=bytes((i*37+width*13+total)%256 for i in range(lx));y=bytes((i*71+width+total*3)%256 for i in range(total-lx));z=bytes(stage(x,y,width))
                rows.append(dict(x=lx,y=total-lx,width=width,offset=len(blob)));blob.extend(x+y+z)
    return {'vectors.bin':bytes(blob),'manifest.json':(json.dumps(dict(vectors=rows,provenance='Own explicit occupied-cell row/column matrix; independent of production index geometry. Byte labels also exercise bit and octet permutations.'),indent=2)+'\n').encode()}
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
