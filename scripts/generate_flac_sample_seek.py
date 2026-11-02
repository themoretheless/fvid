"""Own PCM16 -> seekable FLAC; encoder runs only during fixture generation."""
from pathlib import Path
import struct,json,hashlib,subprocess,tempfile
from generate_vp9_chroma import mp4,frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/flac-sample-seek'
def generate():
 files={'neutral-video.mp4':mp4([frame(8,'420',16,16)]*6,8,'420',(16,16),list(range(1,7)))};cases=[];commands={}
 for rate in [8000,44100,48000]:
  for ch in [1,2]:
   count=4099;values=[((i*157+c*71)%65536)-32768 for i in range(count)for c in range(ch)];raw=b''.join(struct.pack('<h',v)for v in values);base=f'{rate}-{ch}ch';files[base+'.f32le']=b''.join(struct.pack('<f',v/32768)for v in values)
   cmd=['ffmpeg','-hide_banner','-loglevel','error','-f','s16le','-ar',str(rate),'-ac',str(ch),'-i','pipe:0','-threads','1','-map_metadata','-1','-fflags','+bitexact','-flags:a','+bitexact','-c:a','flac','-compression_level','0','-sample_fmt','s16','-f','flac','-y']
   with tempfile.TemporaryDirectory(prefix='fvid-flac-seek-generation-')as directory:
    target=Path(directory)/'final.flac';subprocess.run(cmd+[str(target)],input=raw,check=True);files[base+'.flac']=target.read_bytes()
   commands[base]=cmd+['<seekable-output.flac>'];queries={0,1,500001,1000001}
   for n in [1,17,191,192,575,576,1023,1024,1151,1152,2047,2048,4098,4099]:
    point=n*10**9//rate;queries.update([max(0,point-1),point,point+1])
   cases.append(dict(file=base+'.flac',oracle=base+'.f32le',rate=rate,channels=ch,frames=count,queries_ns=sorted(queries)))
 files['manifest.json']=(json.dumps(dict(provenance='Own mathematical PCM16,independent normalized f32 levels;FFmpeg lossless encoding is generation-only with finalized seekable metadata. Audio-only FLAC uses a separate authored short neutral VP9 video. No private media/codec parameters,network or FFmpeg in ordinary tests.',encoder=subprocess.check_output(['ffmpeg','-version'],text=True).splitlines()[0],commands=commands,cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
