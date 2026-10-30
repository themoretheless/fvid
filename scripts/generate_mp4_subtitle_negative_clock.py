"""Authored VP9/tx3g with signed subtitle composition origins and exact spans."""
from pathlib import Path
import json,struct,hashlib
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/mp4-subtitle-negative-clock'
def main():
    own=generate();parts=list(atoms(own['three-colors.mp4']));assert [k for k,b in parts]==[b'ftyp',b'mdat',b'moov']
    moov=parts[2][1];track=next(b for k,b in atoms(moov) if k==b'trak')
    texts=['visible from zero','before zero','control'];packets=[struct.pack('>H',len(t.encode()))+t.encode() for t in texts]
    def video(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'trak',b'mdia',b'minf',b'stbl']:b=video(b)
            elif k in [b'mvhd',b'mdhd']:b=b[:16]+word(6)+b[20:]
            elif k==b'tkhd':b=b[:20]+word(6)+b[24:]
            elif k==b'stts':b=word(0,1,3,2)
            if k==b'mvhd':b=b[:-4]+word(3)
            out+=atom(k,b)
        return out
    def subtitle(data,offset,scale,shifts,version):
        out=b''
        for k,b in atoms(data):
            if k in [b'mdia',b'minf']:b=subtitle(b,offset,scale,shifts,version)
            elif k==b'tkhd':
                span=max(d+s+n for d,s,n in zip([0,2,3],shifts,[2,1,2]))
                b=b[:12]+word(2)+b[16:20]+word((max(span,0)*50+scale-1)//scale)+b[24:-8]+bytes(8)
            elif k==b'mdhd':b=b[:12]+word(scale,5)+b[20:]
            elif k==b'hdlr':b=word(0,0)+b'sbtl'+bytes(12)+b'Own text\0'
            elif k==b'vmhd':k=b'nmhd';b=word(0)
            elif k==b'stbl':
                entry=atom(b'tx3g',bytes(6)+struct.pack('>H',1)+bytes(30))
                b=atom(b'stsd',word(0,1)+entry)+atom(b'stts',word(0,3,1,2,1,1,1,2))+atom(b'stsz',word(0,0,3,*map(len,packets)))+atom(b'stsc',word(0,1,1,3,1))+atom(b'stco',word(0,1,offset))
                b+=atom(b'ctts',word(version<<24,3)+b''.join(word(1)+struct.pack('>i' if version else '>I',shift) for shift in shifts))
            out+=atom(k,b)
        return out
    prefix=b''.join(atom(k,b) for k,b in parts[:2]);files={f'frame-{i}.yuv16':own[f'frame-{i}.yuv16'] for i in range(3)};cases=[]
    for scale in [50,1001]:
        for name,shifts,version in [('negative',[-1,-4,0],1),('zero',[0,0,0],0),('positive',[1,1,1],0)]:
            name=f'{name}-{scale}.mp4';base=video(moov);size=len(atom(b'moov',base+atom(b'trak',subtitle(track,0,scale,shifts,version))))
            sub=subtitle(track,len(prefix)+size+8,scale,shifts,version)
            files[name]=prefix+atom(b'moov',base+atom(b'trak',sub))+atom(b'mdat',b''.join(packets))
            pts=[d+s for d,s in zip([0,2,3],shifts)];cues=[]
            for text,p,d in zip(texts,pts,[2,1,2]):
                if p+d<=0:continue
                cues.append(dict(text=text,start_ns=max(p,0)*1_000_000_000//scale,end_ns=(p+d)*1_000_000_000//scale))
            cues.sort(key=lambda c:c['start_ns'])
            cases.append(dict(file=name,scale=scale,pts=pts,durations=[2,1,2],texts=texts,cues=cues))
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Pure authored three-color16x16 VP9 and UTF-8 tx3g text. Signed CTTS V1 negative origins, unsigned zero/positive controls, clocks50/1001 Hz. No edits, private media/parameters, FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
