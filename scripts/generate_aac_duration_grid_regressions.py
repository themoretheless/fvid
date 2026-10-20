"""Author AAC-LC silent SCEs and ADTS headers at exact clock boundaries."""
import json,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aac-duration-grid'
def packed(bits):return int(bits+'0'*(-len(bits)%8),2).to_bytes((len(bits)+7)//8,'big')
def field(value,width):return format(value,f'0{width}b')
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    # SCE tag0, gain100, long window, max_sfb0, no predictor/pulse/TNS/gain,
    # no spectral bands, ID_END. All reconstructed coefficients are literal zero.
    payload=packed('000'+'0000'+field(100,8)+'0'+'00'+'0'+'000000'+'0'+'000'+'111')
    for rate,count in [(44100,3),(44100,1),(48000,2),(48000,1),(96000,1),(8000,3)]:
        frequency={96000:0,48000:3,44100:4,8000:11}[rate];size=7+len(payload)
        bits=field(0xfff,12)+'0'+'00'+'1'+'01'+field(frequency,4)+'0'+'001'+'0000'+field(size,13)+field(0x7ff,11)+'00'
        assert len(bits)==56;name=f'lc-{rate}-{count}.aac';files[name]=(packed(bits)+payload)*count
        cases.append(dict(file=name,rate=rate,samples=count*1024,end_ns=count*1024*1000000000//rate))
    files['manifest.json']=(json.dumps(dict(provenance='Six authored mono AAC-LC/ADTS silence streams,max_sfb0,long windows,no prediction/tools. Independent all-zero PCM and integer endpoints. Separate nativeVP9/PCMcompanion,no encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
