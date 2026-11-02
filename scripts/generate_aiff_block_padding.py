"""Generate authored block-aligned AIFF/AIFC acceptance and refusal fixtures."""
from pathlib import Path
import struct, json, hashlib
from generate_g711_fixtures import level
from generate_vp9_chroma import mp4, frame
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/aiff-block-padding'
def chunk(tag, data):
    return tag + struct.pack('>I', len(data)) + data + bytes(len(data) % 2)
def audio(tag, bits, channels, raw, frames, offset, block):
    comm = struct.pack('>HIH', channels, frames, bits) + struct.pack('>HQ', 0x400b, 8000 << 51)
    aifc = tag != b'AIFF'
    if aifc: comm += tag + b'\x00\x00'
    body = (b'AIFC' if aifc else b'AIFF')
    if aifc: body += chunk(b'FVER', struct.pack('>I', 0xa2805140))
    body += chunk(b'COMM', comm) + chunk(b'SSND', struct.pack('>II', offset, block) + bytes([0xcc])*offset + raw)
    return b'FORM' + struct.pack('>I', len(body)) + body

def generate():
    files = {'neutral-video.mp4': mp4([frame(8, '420', 16, 16)]*6, 8, '420', (16,16), list(range(1,7)))}
    cases = []
    for kind, tag, bits, channels in [('int16',b'AIFF',16,1),('int24',b'AIFF',24,2),('float',b'fl32',32,2),('alaw',b'alaw',16,1),('mulaw',b'ulaw',16,2)]:
        count = 2051
        raw = b''; oracle = b''
        for i in range(count):
            for c in range(channels):
                if kind in ['alaw','mulaw']:
                    code = (i + c*71) % 256
                    raw += bytes([code]); value = level(kind,code)/32768
                elif kind == 'float':
                    value = ((i*17+c*31)%257-128)/256
                    raw += struct.pack('>f', value)
                else:
                    value_int = ((i*157+c*83) % (1<<bits)) - (1<<(bits-1))
                    raw += value_int.to_bytes(bits//8, 'big', signed=True); value = value_int/(1<<(bits-1))
                oracle += struct.pack('<f', value)
        files[kind+'.f32le'] = oracle
        for offset in [0,3]:
            pad = (-(offset+len(raw))) % 64
            assert pad > 0
            name = f'{kind}-offset{offset}.aif' + ('c' if tag != b'AIFF' else 'f')
            files[name] = audio(tag,bits,channels,raw+bytes([0x55])*pad,count,offset,64)
            cases.append(dict(file=name,oracle=kind+'.f32le',frames=count,channels=channels,rate=8000,coded_bytes=len(raw),padding_bytes=pad,offset=offset))
        files[kind+'-control.aiff'] = audio(tag,bits,channels,raw,count,0,0)
    raw = bytes.fromhex('80007fff0000')
    files['truncated-declared.aiff'] = audio(b'AIFF',16,1,raw,5,0,64)
    files['incomplete-frame.aiff'] = audio(b'AIFF',16,1,raw+b'\x01',5,0,64)
    files['manifest.json'] = (json.dumps(dict(provenance='Authored PCM/G711, independent mathematical levels, explicit SSND blockSize64, sentinel alignment padding and offset. AIFF/AIFC are audio-only; neutral VP9 companion is a separate valid video control. No private media,FFmpeg or network.',specification='https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/AIFF/Docs/AIFF-1.3.pdf',section='Sound Data Chunk / Block-Aligning Sound Data, pages10-12',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode()
    return files
if __name__ == '__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items(): (OUT/name).write_bytes(data)
