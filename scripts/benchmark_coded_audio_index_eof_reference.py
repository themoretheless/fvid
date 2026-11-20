"""Author compressed/tracker index EOF controls; FFmpeg is generation-only."""
from pathlib import Path
import struct,json,hashlib,subprocess,tempfile
from generate_vp9_chroma import mp4,frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/coded-audio-index-eof'
def generate():
 files={'neutral-video.mp4':mp4([frame(8,'420',16,16)]*6,8,'420',(16,16),list(range(1,7)))};cases=[];commands={}
 for name,kind,options in [('tone.aac','aac',['-c:a','aac','-b:a','64k','-f','adts']),('tone.flac','flac',['-c:a','flac','-compression_level','0','-sample_fmt','s16','-f','flac']),('tone.mp2','mpeg',['-c:a','mp2','-b:a','96k','-f','mp2']),('tone.mp3','mpeg',['-c:a','libmp3lame','-b:a','96k','-write_xing','0','-id3v2_version','0','-f','mp3']),('tone.ac3','dolby',['-c:a','ac3','-b:a','96k','-f','ac3']),('tone.eac3','dolby',['-c:a','eac3','-b:a','96k','-f','eac3'])]:
  command=['ffmpeg','-hide_banner','-loglevel','error','-f','lavfi','-i','sine=frequency=997:sample_rate=48000:duration=0.128','-ac','1','-threads','1','-map_metadata','-1','-fflags','+bitexact','-flags:a','+bitexact',*options,'pipe:1'];files[name]=subprocess.check_output(command);commands[name]=command;cases.append(dict(file=name,kind=kind))
  if kind=='flac':
   files['provisional-streaminfo.flac']=files[name]
   with tempfile.TemporaryDirectory(prefix='fvid-eof-flac-generation-')as directory:
    target=Path(directory)/'final.flac';seekable=command[:-1]+['-y',str(target)];subprocess.check_call(seekable);files[name]=target.read_bytes()
   commands[name]={'pipe_refusal_control':command,'seekable_acceptance':command[:-1]+['-y','<seekable-output.flac>']}
 header=((2047<<21)|(3<<19)|(3<<17)|(1<<16)|(1<<12)|(1<<10)|(3<<6)).to_bytes(4,'big');files['silence.mp1']=(header+bytes(28))*3;cases.append(dict(file='silence.mp1',kind='mpeg'))
 xm=bytearray(336);xm[:17]=b'Extended Module: ';xm[17:25]=b'EOF test';xm[37]=26;xm[38:46]=b'Own fvid';struct.pack_into('<H',xm,58,0x104);struct.pack_into('<I8H',xm,60,276,1,0,1,1,0,1,1,125);xm+=struct.pack('<IBHH',9,0,1,5)+bytes(5);files['silent-row.xm']=bytes(xm);cases.append(dict(file='silent-row.xm',kind='xm'))
 files['manifest.json']=(json.dumps(dict(provenance='Own997Hz sine input,FFmpeg public encoders generation-only;authored silent LayerI allocation-zero frames and one empty XM row. No private media/parameters. Ordinary tests use checked-in bytes without FFmpeg/network. Separate neutral VP9 control for audio-only formats.',refusal_control='provisional-streaminfo.flac',encoder=subprocess.check_output(['ffmpeg','-version'],text=True).splitlines()[0],commands=commands,cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
