"""Author valid FLAC verbatim PCM containing false CRC/metadata/frame boundaries."""
import json,hashlib,struct
from pathlib import Path
from generate_flac_coded_numbers_regressions import frame,stream,crc
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/flac-boundary-collision'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for label,marker in [('clean',b''),('metadata-marker',b'\x81\0\0\4\x12\x34\x56\x78'),('frame-marker',frame(1,64,False,8192))]:
        header=frame(0)[:7];prefix=header+b'\2'+bytes(16);collision=struct.pack('>H',crc(prefix,16,0x8005))+marker if marker else b'';payload=bytes(16)+collision;payload+=bytes(128-len(payload));packet=header+b'\2'+payload;packet+=struct.pack('>H',crc(packet,16,0x8005));second=frame(1,64,False,-4096)
        files[label+'.flac']=stream([packet,second],[64,64]);files[label+'-0.frame']=packet;files[label+'-1.frame']=second;values=list(struct.unpack('>64h',payload))+[-4096]*64;files[label+'.f32le']=b''.join(struct.pack('<f',v/32768.) for v in values);cases.append(dict(file=label+'.flac',packets=[label+'-0.frame',label+'-1.frame'],pcm=label+'.f32le'))
    files['manifest.json']=(json.dumps(dict(provenance='Three valid authored16bitFLACverbatim/constanttracks. PCMcontains a prefixCRC16matching apparentmetadata orcompleteFLACframe header;these are ordinarysamplebytes,notcontainerboundaries. LiteralPCMoracle andindependentwholepacketdecode. VP9/PCMcompanion,no private media/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
