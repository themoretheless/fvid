"""Explicit FFmpeg-only authoring/reference generation; tests load saved bytes."""
import json,hashlib,subprocess,struct,tempfile
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ac3-container-priming'
def boxes(data):
    at=0
    while at<len(data):
        size,kind=struct.unpack_from('>I4s',data,at);assert size>=8 and at+size<=len(data)
        yield kind,data[at+8:at+size];at+=size
    assert at==len(data)
def box(kind,data):return struct.pack('>I4s',len(data)+8,kind)+data
def repeated_audio_edits(data):
    outer=list(boxes(data));kinds=[k for k,_ in outer];assert kinds.index(b'moov')>kinds.index(b'mdat')
    rebuilt=[]
    for kind,body in outer:
        if kind==b'moov':
            children=list(boxes(body));movie=next(b for k,b in children if k==b'mvhd');assert movie[0]==0;scale=struct.unpack_from('>I',movie,12)[0];assert scale%1000==0
            modified=[]
            for k,b in children:
                if k==b'trak':
                    parts=list(boxes(b));mdia=next(v for t,v in parts if t==b'mdia');handler=next(v for t,v in boxes(mdia) if t==b'hdlr')
                    if handler[8:12]==b'soun':
                        entries=[(20*scale//1000,-1),(40*scale//1000,256),(40*scale//1000,256)]
                        edits=box(b'elst',bytes(4)+struct.pack('>I',3)+b''.join(struct.pack('>IiI',duration,start,65536)for duration,start in entries))
                        track=[]
                        for t,v in parts:
                            if t==b'edts':continue
                            if t==b'tkhd':v=bytearray(v);assert v[0]==0;struct.pack_into('>I',v,20,100*scale//1000);v=bytes(v)
                            track.append(box(t,v))
                        b=b''.join(track)+box(b'edts',edits)
                modified.append(box(k,b))
            body=b''.join(modified)
        rebuilt.append(box(kind,body))
    return b''.join(rebuilt)
def generate():
    OUT.mkdir(parents=True,exist_ok=True);cases=[]
    for ext in ['mkv','mp4']:
        path=OUT/('tone.'+ext)
        subprocess.run(['ffmpeg','-hide_banner','-loglevel','error','-y','-f','lavfi','-i','color=c=gray:s=16x16:r=50:d=0.12','-f','lavfi','-i','aevalsrc=0.125*sin(2*PI*440*t):s=48000:d=0.12','-map','0:v:0','-map','1:a:0','-c:v','libvpx-vp9','-lossless','1','-threads','1','-c:a','ac3','-ar','48000','-ac','1','-t','0.12',*(['-movflags','+faststart'] if ext=='mp4' else []),str(path)],check=True)
        ref=OUT/('tone-'+ext+'.f32le');subprocess.run(['ffmpeg','-hide_banner','-loglevel','error','-y','-i',str(path),'-map','0:a:0','-vn','-f','f32le','-c:a','pcm_f32le',str(ref)],check=True)
        probe=json.loads(subprocess.check_output(['ffprobe','-v','error','-select_streams','a:0','-show_entries','stream=sample_rate,channels,duration,start_time:packet=pts,duration,side_data_list','-of','json',str(path)]));cases.append(dict(file=path.name,reference=ref.name,reference_frames=ref.stat().st_size//4,probe=probe))
    with tempfile.TemporaryDirectory(prefix='fvid-ac3-edits-') as directory:
        tail=Path(directory)/'tail.mp4';subprocess.run(['ffmpeg','-hide_banner','-loglevel','error','-y','-i',str(OUT/'tone.mp4'),'-map','0','-c','copy',str(tail)],check=True)
        (OUT/'gap-repeat.mp4').write_bytes(repeated_audio_edits(tail.read_bytes()))
    files=list(OUT.glob('tone*'))+[OUT/'gap-repeat.mp4'];manifest=dict(provenance='Explicitly authored120ms16-square neutral VP9 plus mono440Hz tone, native AC3 in Matroska/MP4. FFmpeg is authoring/reference only; ordinary tests use saved files and no network/encoder. No private media.',cases=cases,sha256={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in files});(OUT/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
if __name__=='__main__':generate()
