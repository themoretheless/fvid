"""Own colored VP9 MP4 with an explicit zero-duration final sample."""
from pathlib import Path
import json,hashlib
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word,mp4
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-empty-tail'
def main():
    controls=generate();packets=[controls[f'frame-{i}.packet'] for i in range(3)];files={}
    def movie(payload):
        base=mp4(payload,8,'420',(16,16),[1,2,3])
        def write(data):
            out=b''
            for k,b in atoms(data):
                if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=write(b)
                elif k==b'stts':b=word(0,2,2,1,1,0)
                elif k in [b'mvhd',b'mdhd']:b=b[:16]+word(2)+b[20:]
                elif k==b'tkhd':b=b[:20]+word(2)+b[24:]
                out+=atom(k,b)
            return out
        return write(base)
    files['empty-tail.mp4']=movie(packets)
    files['invalid-empty-tail.mp4']=movie(packets[:2]+[b'\0'])
    files.update({n:b for n,b in controls.items() if n.endswith('.yuv16')})
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Owned three colored VP9 keyframes, explicit stts durations [1,1,0] at 50Hz. Media/movie/track durations are two ticks. Corrupt control has an invalid VP9 frame marker solely in the final zero-duration packet. No private media, encoder or network.',duration_ticks=2,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
