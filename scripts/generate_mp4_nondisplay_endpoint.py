"""Owned VP9 hidden-frame composition bounds; offline authored controls."""
from pathlib import Path
import hashlib,json,struct
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-nondisplay-endpoint'
def main():
    files={};cases=[]
    def add(name,hidden,pts,version=0,movie_scale=50,header_duration=3,display_duration=3,marker=None):
        packets=[frame(8,'420',16,16,show=i!=hidden) for i in range(3)]
        base=mp4(packets,8,'420',(16,16),[1,2,3])
        raw_header=header_duration if marker is None else marker
        def write(data):
            out=b''
            for k,b in atoms(data):
                if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                    b=write(b)
                    if k==b'stbl':b+=atom(b'ctts',word(0,3)+b''.join(word(1,p-i) for i,p in enumerate(pts)))
                elif k==b'mvhd':b=b[:12]+word(movie_scale,max(1,(display_duration*movie_scale+49)//50))+b[20:]
                elif k==b'tkhd':
                    if version==0:b=b[:20]+word(raw_header)+b[24:]
                    else:b=word(0x01000000|int.from_bytes(b[:4],'big'))+struct.pack('>QQIIQ',int.from_bytes(b[4:8],'big'),int.from_bytes(b[8:12],'big'),int.from_bytes(b[12:16],'big'),0,raw_header)+b[24:]
                out+=atom(k,b)
            return out
        files[name]=write(base)
        shown=sorted(p for i,p in enumerate(pts) if i!=hidden)
        intervals=[[p,shown[i+1] if i+1<len(shown) else p+1] for i,p in enumerate(shown)]
        known=None if raw_header in [0,(1<<(32 if version==0 else 64))-1] else raw_header
        cases.append(dict(file=name,hidden=hidden,intervals=intervals,duration=display_duration,header_duration=known,movie_scale=movie_scale))
    for version in [0,1]:
        for hidden in range(3):
            pts=[0,1,2];pts[hidden]=100
            name=f'hidden-{hidden}'+('' if version==0 else '-tkhd-v1')+'.mp4'
            add(name,hidden,pts,version)
        for header in [2,3]:add(f'coarse-v{version}-duration-{header}.mp4',None,[2,3,4],version,25,header,5)
        for label,marker in [('zero',0),('unknown',(1<<(32 if version==0 else 64))-1)]:
            add(f'header-v{version}-{label}.mp4',None,[2,3,4],version,50,0,5,marker)
            for hidden in range(3):
                pts=[0,1,2];pts[hidden]=100
                add(f'hidden-{hidden}-v{version}-{label}.mp4',hidden,pts,version,50,0,3,marker)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Own neutral VP9 with explicit show_frame flags. Hidden frames at late PTS; known/zero/unknown V0/V1 tkhd and positive-CTS coarse-clock controls. Raw media decode duration stays 3 ticks at 50Hz. No private media, encoder or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
