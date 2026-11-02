"""Authored 1..32-bit left-justified AIFF/AIFC PCM, generation only."""
from pathlib import Path
import struct,json,hashlib
from generate_aiff_block_padding import audio
from generate_vp9_chroma import mp4,frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aiff-integer-precision'

def generate():
    files={'neutral-video.mp4':mp4([frame(8,'420',16,16)]*6,8,'420',(16,16),list(range(1,7)))};cases=[]
    for bits in range(1,33):
        width=(bits+7)//8;shift=width*8-bits;half=1<<(bits-1)
        for channels in [1,2]:
            integers=[];expected=b''
            edges=[-half,half-1,0,-1,min(1,half-1)]
            for i in range(2051):
                for c in range(channels):
                    value=edges[(i+c)%len(edges)] if i<20 else ((i*157+c*71)%(2*half))-half
                    integers.append(value);expected+=struct.pack('<f',value/half)
            oracle=f'pcm-{bits}bit-{channels}ch.f32le';files[oracle]=expected
            for coding in ['AIFF','NONE','sowt']:
                order='little' if coding=='sowt' else 'big'
                raw=b''.join((v<<shift).to_bytes(width,order,signed=True) for v in integers)
                name=f'{coding}-{bits}bit-{channels}ch.aiff'
                files[name]=audio(coding.encode(),bits,channels,raw,2051,0,0)
                cases.append(dict(file=name,oracle=oracle,bits=bits,channels=channels,stored_bytes=width,frames=2051,coding=coding))
    for bits in [0,33,65535]:files[f'invalid-{bits}bit.aiff']=audio(b'AIFF',bits,1,b'\x00\x00',1,0,0)
    files['unused-low-bits-big.aiff']=audio(b'AIFF',12,1,b'\x00\x01',1,0,0)
    files['unused-low-bits-little.aiff']=audio(b'sowt',23,2,b'\x01\x00\x00'*2,1,0,0)
    files['orphan-channel.aiff']=audio(b'AIFF',12,2,b'\x00'*7,2,0,0)
    files['truncated-declared.aiff']=audio(b'AIFF',12,1,b'\x00\x00'*3,4,0,0)
    # Alignment tail is storage padding, not unused bits of valid sample points.
    files['padded-12bit.aiff']=audio(b'AIFF',12,1,b'\x80\x00\x7f\xf0'+b'\x55'*60,2,0,64)
    files['manifest.json']=(json.dumps(dict(provenance='Own mathematical signed PCM values, left justification and zero unused bits; all widths1..32,mono/stereo,AIFF and AIFC NONE/sowt. No private media/parameters,FFmpeg or network. Audio-only containers use a separate authored neutral VP9 control.',specification='https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/AIFF/Docs/AIFF-1.3.pdf',section='Common Chunk and Sample Points,pages9-11',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
