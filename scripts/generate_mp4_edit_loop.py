"""Owned finite/looped VP9 edit-list companions; no external encoder."""
from pathlib import Path
import hashlib,json,struct
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import atom,word,mp4
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-edit-loop'
def main():
    controls=generate();packets=[controls[f'frame-{i}.packet'] for i in range(3)]
    base=mp4(packets,8,'420',(16,16),[1,2,3])
    def build(version,flags):
        row=word(2,1,65536) if version==0 else struct.pack('>QqI',2,1,65536)
        def rewrite(data):
            out=b''
            for k,b in atoms(data):
                if k in [b'moov',b'trak']:
                    b=rewrite(b)
                    if k==b'trak':b+=atom(b'edts',atom(b'elst',word(version<<24|flags,1)+row))
                elif k==b'mvhd':b=b[:16]+word(2)+b[20:]
                elif k==b'tkhd':b=b[:20]+word(2)+b[24:]
                out+=atom(k,b)
            return out
        return rewrite(base)
    files={f'v{v}-{name}.mp4':build(v,flags) for v in [0,1] for name,flags in [('finite',0),('loop',1),('reserved',2),('loop-reserved',3)]}
    for i in [1,2]:files[f'frame-{i}.yuv16']=controls[f'frame-{i}.yuv16']
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Own three colored VP9 keyframes, 50 Hz; two-frame trim. V0/V1 edit flags only distinguish finite, loop and reserved cases. No private media/parameters, FFmpeg or network.','sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
