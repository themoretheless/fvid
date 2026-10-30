"""Owned short edit programs whose media windows include decode-only data."""
from pathlib import Path
import json,hashlib,struct
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-edit-window-eof'
def main():
    controls=generate();colors=[controls[f'frame-{i}.packet'] for i in range(3)];hidden=frame(8,'420',16,16,show=False)
    files={'source-tail.mp4':mp4(colors[:2]+[hidden],8,'420',(16,16),[1,2,3]),'source-middle.mp4':mp4([colors[0],hidden,colors[1]],8,'420',(16,16),[1,2,3]),'source-leading.mp4':mp4([hidden]+colors[:2],8,'420',(16,16),[1,2,3])}
    def edit(data,rows):
        end=sum(d for d,_ in rows)
        def write(data):
            out=b''
            for k,b in atoms(data):
                if k in [b'moov',b'trak']:
                    b=write(b)
                    if k==b'trak':b+=atom(b'edts',atom(b'elst',word(0,len(rows))+b''.join(struct.pack('>IiI',d,m,65536) for d,m in rows)))
                elif k==b'mvhd':b=b[:16]+word(end)+b[20:]
                elif k==b'tkhd':b=b[:20]+word(end)+b[24:]
                out+=atom(k,b)
            return out
        return write(data)
    cases=[]
    def add(name,source,rows,frames):
        files[name]=edit(files[source],rows);cases.append(dict(file=name,source=source,duration=sum(d for d,_ in rows),frames=[dict(start=a,end=b,color=c) for a,b,c in frames]))
    add('tail-repeat.mp4','source-tail.mp4',[(3,0),(3,0)],[(0,1,0),(1,2,1),(3,4,0),(4,5,1)])
    add('tail-empty-first.mp4','source-tail.mp4',[(1,2),(2,0)],[(1,2,0),(2,3,1)])
    add('tail-reordered.mp4','source-tail.mp4',[(1,1),(3,0)],[(0,1,1),(1,2,0),(2,3,1)])
    add('middle-trim-repeat.mp4','source-middle.mp4',[(2,1),(2,1)],[(0,1,0),(1,2,1),(2,3,0),(3,4,1)])
    add('leading-trailing-empty.mp4','source-leading.mp4',[(2,1),(1,0)],[(0,1,0),(1,2,1)])
    add('leading-empty-first.mp4','source-leading.mp4',[(1,0),(2,1)],[(1,2,0),(2,3,1)])
    files['all-hidden-many.mp4']=edit(mp4([hidden]*3,8,'420',(16,16),[1,2,3]),[(3,0)]*8)
    def preroll(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                b=preroll(b)
                if k==b'stbl':b+=atom(b'ctts',word(1<<24,1,3)+struct.pack('>i',-4))
            out+=atom(k,b)
        return out
    files['preroll-many.mp4']=edit(preroll(mp4(colors,8,'420',(16,16),[1,2,3])),[(3,0)]*8)
    files['corrupt-window.mp4']=edit(mp4(colors[:2]+[b'\0'],8,'420',(16,16),[1,2,3]),[(1,2),(2,0)])
    for i in range(2):files[f'frame-{i}.yuv16']=controls[f'frame-{i}.yuv16']
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Own three-sample VP9 at 50 Hz, colored shown pictures and hidden keyframes. Authored finite media edits with independent normalized frame intervals; no private media, external encoder or network.',cases=cases,no_picture=['all-hidden-many.mp4','preroll-many.mp4'],invalid='corrupt-window.mp4',sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
