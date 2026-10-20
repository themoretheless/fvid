"""Author FLAC8bitexplicit-rate255kHz versus16bit10Hz encoding."""
import json,hashlib,struct
from pathlib import Path
from generate_flac_coded_numbers_regressions import frame,stream
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/flac-rate-escape'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for rate in [1000,8000,254000,255000]:
        label=f'rate-{rate}';packets=[frame(i,64,False,level,rate_code=12,rate_extra=bytes([rate//1000])) for i,level in enumerate([4096,-4096])];control=[frame(i,64,False,level,rate_code=14,rate_extra=struct.pack('>H',rate//10)) for i,level in enumerate([4096,-4096])];files[label+'.flac']=stream(packets,[64,64],rate);files[label+'-10hz.flac']=stream(control,[64,64],rate);names=[]
        for i,packet in enumerate(packets):name=f'{label}-{i}.frame';files[name]=packet;names.append(name)
        cases.append(dict(file=label+'.flac',control=label+'-10hz.flac',size=64,rate=rate,levels=[4096,-4096],packets=names))
    packet=frame(0,64,rate_code=12,rate_extra=b'\0');files['invalid-zero.frame']=packet;files['invalid-zero.flac']=stream([packet],[64],8000)
    files['manifest.json']=(json.dumps(dict(provenance='Four authored constant16bitFLACstreams compare8bitkHz versus16bit10Hz explicitrateheaders at1/8/254/255kHz.255islegal8bitvalue,notreserved. LiteralPCM/nativepacketcontrols andzero-header-rate geometryrefusal. VP9/PCMcompanion,no private media/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
