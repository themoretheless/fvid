"""Owned VP9 decode-only tail witnesses for seek landing recovery."""
from pathlib import Path
import hashlib,json,struct
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-decode-only-seek'
def main():
    colors=generate();hidden=frame(8,'420',16,16,show=False);files={}
    def movie(packets,shift=0):
        base=mp4(packets,8,'420',(16,16),[1,2,3])
        if not shift:return base
        def write(data):
            out=b''
            for k,b in atoms(data):
                if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                    b=write(b)
                    if k==b'stbl':b+=atom(b'ctts',word(0,3,1,0,1,0,1,shift))
                out+=atom(k,b)
            return out
        return write(base)
    visible=[colors['frame-0.packet'],colors['frame-1.packet']]
    files['ordinary-tail.mp4']=movie(visible+[hidden])
    files['late-tail.mp4']=movie(visible+[hidden],98)
    files['middle-control.mp4']=movie([visible[0],hidden,visible[1]])
    files['leading-control.mp4']=movie([hidden]+visible)
    def edited(data, rows):
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
    files['middle-trim-edit.mp4']=edited(files['middle-control.mp4'],[(2,1)])
    files['middle-repeat-edit.mp4']=edited(files['middle-control.mp4'],[(3,0),(3,0)])
    files['all-hidden.mp4']=movie([hidden]*3)
    files['corrupt-tail.mp4']=movie(visible+[b'\0'])
    for i in range(2):files[f'frame-{i}.yuv16']=colors[f'frame-{i}.yuv16']
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Own three-sample VP9 movies at 50Hz: two distinct colored shown keyframes, a hidden keyframe at tail/middle/start, normalized trim/repeat edit views, all-hidden and corrupt-marker controls. No private media, encoder or network.',duration_ticks=3,case_intervals={'ordinary-tail.mp4':[[0,1],[1,2]],'late-tail.mp4':[[0,1],[1,2]],'middle-control.mp4':[[0,2],[2,3]],'leading-control.mp4':[[1,2],[2,3]],'middle-trim-edit.mp4':[[0,1],[1,2]],'middle-repeat-edit.mp4':[[0,2],[2,3],[3,5],[5,6]]},no_picture=['all-hidden.mp4'],invalid=['corrupt-tail.mp4'],sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
