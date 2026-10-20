"""Author FLAC block-size code acceptance against explicit escape controls."""
import json,hashlib
from pathlib import Path
from generate_flac_coded_numbers_regressions import frame,stream
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/flac-block-codes'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for label,code,size,count,control_code in [('escape-256',6,256,2,8),('code13',13,8192,2,7),('code14',14,16384,2,7),('code12',12,4096,2,7),('code15',15,32768,2,7),('maximum65535',7,65535,1,7)]:
        levels=[4096,-4096][:count];packets=[frame(i,size,False,levels[i],block_code=code) for i in range(count)];controls=[frame(i,size,False,levels[i],block_code=control_code) for i in range(count)];files[label+'.flac']=stream(packets,[size]*count);files[label+'-escape.flac']=stream(controls,[size]*count);cases.append(dict(file=label+'.flac',control=label+'-escape.flac',size=size,levels=levels,packets=[]))
        for i,packet in enumerate(packets):name=f'{label}-{i}.frame';files[name]=packet;cases[-1]['packets'].append(name)
    files['invalid-65536.frame']=frame(0,65536,block_code=7)
    files['invalid-reserved.frame']=frame(0,block_code=0)
    files['manifest.json']=(json.dumps(dict(provenance='Six authored constant16bitFLACblock-size descriptors:legal8192/16384codes13/14and8bitescape255representing256,versusequivalent explicit/implicitcontrols. Healthy4096/32768/65535bounds. LiteralPCMlevels andindependent nativepacketdecode. VP9/PCMcompanion,no private data/encoder/network.',cases=cases,invalid=['invalid-65536.frame','invalid-reserved.frame'],sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
