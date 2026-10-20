"""Author silent Vorbis and linked Vorbis/Opus Ogg duration boundaries."""
import json,hashlib,struct
from pathlib import Path
from generate_vorbis_timestamps import bits,page
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ogg-duration-grid'
def vorbis(rate,end,serial):
    ident=b'\x01vorbis'+struct.pack('<IBIiiiBB',0,2,rate,0,0,0,0x66,1)
    comment=b'\x03vorbis'+bytes(8)+b'\x01'
    fields=[(5,8)]+[(b,8)for b in b'vorbis']+[(0,8),(0x564342,24),(1,16),(1,24),(0,1),(0,1),(0,5),(0,4),(0,6),(0,16),(0,6),(1,16),(0,5),(0,2),(5,4),(0,6),(0,16),(0,24),(32,24),(31,24),(0,6),(0,8),(0,3),(0,1),(0,6),(0,16),(0,1),(0,1),(0,2),(0,8),(0,8),(0,8),(0,6)]
    for long in [0]:fields.extend([(long,1),(0,16),(0,16),(0,8)])
    setup=bits(fields+[(1,1)]);out=page(ident,0,2,0,serial)+page(comment,0,0,1,serial)+page(setup,0,0,2,serial)
    modes=[0]*15
    for i,mode in enumerate(modes):
        header=[(0,1)]
        if mode==1:header.extend([(int(i>0 and modes[i-1]==1),1),(int(i+1<len(modes)and modes[i+1]==1),1)])
        payload=bits(header+[(0,1),(0,1)])
        out+=page(payload,end if i==14 else 32 if i==1 else -1,4 if i==14 else 0,i+3,serial)
    return out

def opus(serial):
    head=b'OpusHead'+bytes([1,2])+struct.pack('<HIhB',0,48000,0,0)
    tags=b'OpusTags'+bytes(8)
    return page(head,0,2,0,serial)+page(tags,0,0,1,serial)+page(bytes.fromhex('f8fffe'),960,4,2,serial)
def generate():
    files={'video.mkv':pcm()['video.mkv'],'opus-control.ogg':opus(0x46564730)};cases=[]
    for rate,end in [(48000,448),(48000,446),(44100,448),(44100,447),(96000,448),(8000,448)]:
        name=f'vorbis-{rate}-{end}.ogg';files[name]=vorbis(rate,end,0x46564710);cases.append(dict(file=name,kind='vorbis',rate=rate,samples=end,end_ns=end*1000000000//rate))
    for end in [448,446]:
        name=f'joint-{end}.ogg';files[name]=vorbis(48000,end,0x46564720)+opus(0x46564721);total=end+960;cases.append(dict(file=name,kind='joint',rate=48000,samples=total,end_ns=total*1000000000//48000))
    files['manifest.json']=(json.dumps(dict(provenance='Eight authored silent Vorbis /linkedVorbis+Opus Ogg programs with independent zero PCM,EOSsample totals andinteger endpoints. Vorbis sparse single-codebook,uniform64sample blocks,no activefloor;OpusDTXzero preskip. NativeVP9/PCMcompanion;no encoder/network/private data.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
