"""Authored single PCM stream with nanosecond container timestamps."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_matroska_enabled_track_regressions import video
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-pcm-clock'
def clip(codec=b'A_PCM/INT/LIT', offset=0, padding=(0,0,0), codec_delay_ns=0):
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    info=ebml('1549a966',ebml('2ad7b1',(1_000_000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',60.)))
    audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',codec)+ebml('23e383',(20_000_000).to_bytes(4,'big'))+ebml('e1',ebml('b5',struct.pack('>d',8000.))+ebml('9f',b'\1')+ebml('6264',bytes([32 if codec.endswith(b'IEEE') else 16]))))
    if codec_delay_ns:
        assert audio[1]&0xf0==0x10
        audio=ebml('ae',audio[5:]+ebml('56aa',codec_delay_ns.to_bytes(8,'big')))
    blocks=ebml('e7',b'\0');expected=bytearray()
    for index in range(3):
        values=[((index*160+i)*37)%32768-16384 for i in range(160)]
        expected.extend(b''.join(struct.pack('<f',v/32768.) for v in values))
        for number,payload in [(1,frame(8,'420',16,16)),(2,b''.join(struct.pack('<f',v/32768.) if codec.endswith(b'IEEE') else struct.pack('>h' if codec.endswith(b'BIG') else '<h',v) for v in values))]:
            group=ebml('a1',bytes([0x80|number])+struct.pack('>h',index*20+(offset if number==2 else 0))+b'\0'+payload)+ebml('9b',b'\x14')
            if number==2 and padding[index]:group+=ebml('75a2',padding[index].to_bytes(8,'big',signed=True))
            blocks+=ebml('a0',group)
    return header+ebml('18538067',info+ebml('1654ae6b',video(1,16,b'\1')+audio)+ebml('1f43b675',blocks)),bytes(expected)
def generate():
    files={};cases=[]
    for name,codec,offset in [('clock',b'A_PCM/INT/LIT',0),('big',b'A_PCM/INT/BIG',0),('float',b'A_PCM/FLOAT/IEEE',0),('negative',b'A_PCM/INT/LIT',-10)]:
        data,expected=clip(codec,offset)
        files[name+'.mkv']=data
        files[name+'.f32le']=expected[80*4:] if offset<0 else expected
        cases.append({'file':name+'.mkv','oracle':name+'.f32le','offset_ms':offset})
    files['expected-audio.f32le']=files['clock.f32le']
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms16-square VP9 and8kHz mono PCM in little/big16bit or float32. Three160-sample packets at0/20/40ms; negative companion shifts audio by-10ms. No private media, encoder or network.','cases':cases,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
