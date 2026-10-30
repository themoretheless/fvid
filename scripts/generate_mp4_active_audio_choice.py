"""Own preferred PCM track with unselected unsupported setup and quiet alternate."""
from pathlib import Path
import json,struct,hashlib
from generate_mp4_enabled_track_regressions import audiovisual
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-active-audio-choice'
def main():
    source,pcm=audiovisual();files={'control.mp4':source,'expected.f32le':pcm}
    def build(first=True,selected_bad=False,compact=False):
        def rewrite(data,preferred=False):
            out=b''
            for k,b in atoms(data):
                active=preferred
                if k==b'trak':active=struct.unpack_from('>I',dict(atoms(b))[b'tkhd'],12)[0]==3
                if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=rewrite(b,active)
                elif k==b'stsd' and preferred:
                    entries=list(atoms(b[8:]));assert len(entries)==1
                    kind,body=entries[0];bad=bytearray(body);bad[18:20]=struct.pack('>H',272)
                    b=word(0,2)+(atom(kind,bad)+atom(kind,body) if first else atom(kind,body)+atom(kind,bad))
                elif k==b'stsc' and preferred:b=word(0,1,1,3,1 if selected_bad or not first else 2)
                elif k==b'stsz' and preferred and compact:b=word(0,320,3)
                out+=atom(k,b)
            return out
        return rewrite(source)
    files['unused-first.mp4']=build();files['unused-last.mp4']=build(first=False);files['unused-first-compact.mp4']=build(compact=True);files['selected-bad.mp4']=build(selected_bad=True)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Own 60ms VP9 and two 8kHz mono PCM tracks; preferred enabled track has analytic nonzero samples, alternate is silence. Unsupported 272-bit PCM setup is authored only as an unselected entry, matching the existing PCM-choice capability-control convention. No private media/parameters, external encoder, FFmpeg or network.','sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
