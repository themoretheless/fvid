"""Authored VP9/tx3g movie edits with exact rational presentation oracles."""
from pathlib import Path
from fractions import Fraction
import json,struct,hashlib
from generate_mp4_seek_io_regressions import generate
from generate_mp4_subtitle_negative_clock import text_entry
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-subtitle-edits'
def main():
    own=generate();parts=list(atoms(own['three-colors.mp4']));prefix=b''.join(atom(k,b) for k,b in parts[:2]);texts=['First caption','Middle caption','Last caption'];packets=[struct.pack('>H',len(t))+t.encode() for t in texts]
    def video(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'trak',b'mdia',b'minf',b'stbl']:b=video(b)
            elif k in [b'mvhd',b'mdhd']:b=b[:12]+word(1000,120)+b[20:]
            elif k==b'tkhd':b=b[:20]+word(120)+b[24:]
            elif k==b'stts':b=word(0,1,3,40)
            if k==b'mvhd':b=b[:-4]+word(3)
            out+=atom(k,b)
        return out
    base=video(parts[2][1]);vtrack=next(b for k,b in atoms(base) if k==b'trak')
    def sub(data,offset,scale,rows,version=0,bad=False,with_edit=True,overlap=False):
        out=b''
        for k,b in atoms(data):
            if k in [b'mdia',b'minf']:b=sub(b,offset,scale,rows,version,with_edit=False,overlap=overlap)
            elif k==b'tkhd':b=b[:12]+word(2)+b[16:20]+word(120)+b[24:]
            elif k==b'mdhd':b=b[:12]+word(scale,120)+b[20:]
            elif k==b'hdlr':b=word(0,0)+b'sbtl'+bytes(12)+b'Authored text\0'
            elif k==b'vmhd':k=b'nmhd';b=word(0)
            elif k==b'stbl':b=atom(b'stsd',word(0,1)+text_entry())+atom(b'stts',word(0,3,1,60,1,40,1,20) if overlap else word(0,1,3,40))+atom(b'stsz',word(0,0,3,*map(len,packets)))+atom(b'stsc',word(0,1,1,3,1))+atom(b'stco',word(0,1,offset))
            if k==b'stbl' and overlap:b+=atom(b'ctts',word(1<<24,3)+b''.join(struct.pack('>Ii',1,p) for p in [0,-60,-100]))
            out+=atom(k,b)
        if with_edit and rows is not None:
            payload=word(version<<24,len(rows)+(1 if bad else 0))+b''.join(struct.pack('>QqI' if version else '>IiI',d,m,65536) for d,m in rows)
            out+=atom(b'edts',atom(b'elst',payload))
        return out
    def clip(scale,rows,version=0,bad=False,video_rows=None,overlap=False):
        movie=base
        if video_rows is not None:
            movie=b''.join(atom(k,b+atom(b'edts',atom(b'elst',word(0,len(video_rows))+b''.join(struct.pack('>IiI',d,m,65536) for d,m in video_rows))) if k==b'trak' else b) for k,b in atoms(base))
        if rows is not None and len(rows)==1 and rows[0][0]==0:
            movie+=atom(b'mvex',atom(b'trex',word(0,2,1,40,0,0)))
        if rows is not None and len(rows)==1 and rows[0][0]==0:
            def empty_tables(data):
                out=b''
                for k,b in atoms(data):
                    if k in [b'mdia',b'minf',b'stbl']:b=empty_tables(b)
                    elif k in [b'stts',b'stsc',b'stco']:b=word(0,0)
                    elif k==b'stsz':b=word(0,0,0)
                    out+=atom(k,b)
                return out
            text_track=empty_tables(sub(vtrack,0,scale,rows,version,bad,overlap=overlap))
            def fragment(offset):
                traf=atom(b'tfhd',word(0x020008,2,40))+atom(b'tfdt',word(0,0))+atom(b'trun',word(0x201,3)+struct.pack('>i',offset)+word(*map(len,packets)))
                return atom(b'moof',atom(b'mfhd',word(0,1))+atom(b'traf',traf))
            moof=fragment(0);moof=fragment(len(moof)+8)
            return prefix+atom(b'moov',movie+atom(b'trak',text_track))+moof+atom(b'mdat',b''.join(packets))
        size=len(atom(b'moov',movie+atom(b'trak',sub(vtrack,0,scale,rows,version,bad,overlap=overlap))))
        return prefix+atom(b'moov',movie+atom(b'trak',sub(vtrack,len(prefix)+size+8,scale,rows,version,bad,overlap=overlap)))+atom(b'mdat',b''.join(packets))
    programs={'none':None,'open':[(0,40)],'identity':[(120,0)],'delay':[(40,-1),(80,0)],'trim':[(40,10),(80,-1)],'gap':[(40,0),(40,-1),(40,80)],'reorder':[(40,80),(40,0),(40,40)],'repeat':[(40,0),(40,0),(40,40)],'blank':[(120,-1)],'short-tail':[(10,110),(110,-1)],'bounded-repeat':[(10,m) for m in [0,40]*4]+[(40,-1)]}
    files={f'frame-{i}.yuv16':own[f'frame-{i}.yuv16'] for i in range(3)};cases=[]
    for scale in [1000,1001]:
        for name,rows in programs.items():
            version=1 if name in ['reorder','repeat'] else 0;n=f'{name}-{scale}.mp4';files[n]=clip(scale,rows,version);cues=[]
            for i,t in enumerate(texts):files[f'text-{i}.packet']=packets[i]
            if rows is None:
                cues=[dict(text=t,start_ns=p*1_000_000_000//scale,end_ns=(p+40)*1_000_000_000//scale) for t,p in zip(texts,[0,40,80])]
            else:
                movie=Fraction(0)
                for duration,media in rows:
                    span=Fraction(duration,1000) if duration else Fraction(120-media,scale)
                    if media>=0:
                        for text,p in zip(texts,[0,40,80]):
                            a=max(Fraction(p-media,scale),Fraction(0));b=min(Fraction(p+40-media,scale),span)
                            if b>a:cues.append(dict(text=text,start_ns=int((movie+a)*1_000_000_000),end_ns=int((movie+b)*1_000_000_000)))
                    movie+=span
            cues.sort(key=lambda c:(c['start_ns'],c['end_ns']));cases.append(dict(file=n,scale=scale,movie_scale=1000,edits=rows,version=version,texts=texts,cues=cues,resource_bound=name=='bounded-repeat'))
    for scale in [1000,1001]:
        for name,rows in [('matched-video-delay',[(40,-1),(80,0)]),('video-only-delay',None)]:
            n=f'{name}-{scale}.mp4';video_rows=[(40,-1),(80,0)];files[n]=clip(scale,rows,video_rows=video_rows);cues=[]
            program=rows if rows is not None else [(120,0)];movie=Fraction(0)
            for duration,media in program:
                if media>=0:
                    for text,p in zip(texts,[0,40,80]):
                        a=max(Fraction(p-media,scale),Fraction(0));b=min(Fraction(p+40-media,scale),Fraction(duration,1000))
                        start=movie+a-Fraction(40,1000);end=movie+b-Fraction(40,1000)
                        if b>a and end>0:cues.append(dict(text=text,start_ns=int(max(start,Fraction(0))*1_000_000_000),end_ns=int(end*1_000_000_000)))
                movie+=Fraction(duration,1000)
            cues.sort(key=lambda c:(c['start_ns'],c['end_ns']));cases.append(dict(file=n,scale=scale,movie_scale=1000,edits=rows,video_edits=video_rows,video_duration_ns=80_000_000,version=0,texts=texts,cues=cues,resource_bound=False))
    for scale in [1000,1001]:
        n=f'coincident-{scale}.mp4';rows=[(120,0)];files[n]=clip(scale,rows,overlap=True)
        cues=[dict(text=t,start_ns=0,end_ns=d*1_000_000_000//scale) for t,d in zip(texts,[60,40,20])]
        cases.append(dict(file=n,scale=scale,movie_scale=1000,edits=rows,version=0,texts=texts,cues=cues,pts=[0,0,0],durations=[60,40,20],resource_bound=False,coincident=True))
    for scale in [997,1001]:
        rows=[(1,-1),(120,0)]
        def fractional_movie(data,offset,text=False):
            out=b''
            for k,b in atoms(data):
                if k==b'trak':
                    header=next(v for t,v in atoms(b) if t==b'tkhd')
                    b=fractional_movie(b,offset,int.from_bytes(header[12:16],'big')==2)
                elif k in [b'mdia',b'minf',b'stbl']:b=fractional_movie(b,offset,text)
                elif k==b'mvhd':b=b[:12]+word(1001,1001)+b[20:]
                elif k==b'tkhd':b=b[:20]+word(121 if text else 1001)+b[24:]
                elif k==b'mdhd' and not text:b=b[:12]+word(1000,1000)+b[20:]
                elif k==b'stts' and not text:b=word(0,3,1,400,1,400,1,200)
                elif k==b'stco' and text:b=word(0,1,offset)
                out+=atom(k,b)
            return out
        raw=base+atom(b'trak',sub(vtrack,0,scale,rows))
        movie=fractional_movie(raw,0);size=len(atom(b'moov',movie));movie=fractional_movie(raw,len(prefix)+size+8)
        n=f'fractional-origin-{scale}.mp4';files[n]=prefix+atom(b'moov',movie)+atom(b'mdat',b''.join(packets));cues=[]
        for text,p in zip(texts,[0,40,80]):
            a=Fraction(p,scale);b=min(Fraction(p+40,scale),Fraction(120,1001))
            if b>a:cues.append(dict(text=text,start_ns=int((Fraction(1,1001)+a)*1_000_000_000),end_ns=int((Fraction(1,1001)+b)*1_000_000_000)))
        cases.append(dict(file=n,scale=scale,movie_scale=1001,edits=rows,version=0,texts=texts,cues=cues,resource_bound=False,video_duration_ns=1_000_000_000,video_pts_ticks=[0,400,800],video_durations_ticks=[400,400,200],video_intervals_ns=[[0,400_000_000],[400_000_000,800_000_000],[800_000_000,1_000_000_000]]))
    files['bad-edit-count.mp4']=clip(1000,[(120,0)],bad=True)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Pure authored three-color16x16 VP9 and complete visible tx3g text entry. Video stays unedited at0/40/80ms. Subtitle-only v0/v1 elst programs use independent Fraction oracles, differing track/movie scales, delay/trim/blank/reorder/repeat/end and bounded expansion. No private media/parameters, FFmpeg or network.',reference='https://developer.apple.com/documentation/quicktime-file-format/edit_list_atom',cases=cases,refused=['bad-edit-count.mp4'],sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
