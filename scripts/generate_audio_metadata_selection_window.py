"""Authored short Opus/Vorbis and PCM for metadata-only stream selection."""
from pathlib import Path
import struct,json,hashlib
from generate_wave_ogg_flac_plan_regressions import ogg_page,make_vorbis_link
from generate_remux_concat_controls_regressions import make_pcm_wave
from generate_wave_ogg_flac_plan_regressions import make_g711_wave
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-metadata-selection-window'
def generate():
    head=b'OpusHead'+struct.pack('<BBHIhB',1,1,0,48000,0,0);vendor=b'fvid-authored-metadata'*4;tags=b'OpusTags'+struct.pack('<I',len(vendor))+vendor+struct.pack('<I',0)
    def opus(packets):
        result=ogg_page(head,0,2,0,810)+ogg_page(tags,0,0,1,810)
        for i,p in enumerate(packets):result+=ogg_page(p,(i+1)*960,4 if i+1==len(packets)else 0,i+2,810)
        return result
    samples=[(i%17-8)*73 for i in range(960)]
    files={'one.ogg':opus([bytes.fromhex('f8fffe000000')]),'tail.ogg':opus([bytes.fromhex('f8fffe000000'),bytes.fromhex('f8fffe000000000000')]),'vorbis.ogg':make_vorbis_link(serial=811,channels=1),'front.wav':make_pcm_wave(48000,samples),'front.s16le':struct.pack('<960h',*samples),'front.f32le':b''.join(struct.pack('<f',x/32768)for x in samples)}
    codes=bytes(0xd5 if i%2==0 else 0x55 for i in range(960))
    files['front-alaw.wav']=make_g711_wave(law='alaw',sample_rate=48000,channels=1,codes=codes)
    files['front-alaw.f32le']=b''.join(struct.pack('<f',(8 if i%2==0 else -8)/32768)for i in range(960))
    files['manifest.json']=(json.dumps(dict(provenance='Authored20/40ms mono48kHz Opus silence with oversized metadata and a larger valid second packet,authored short silent Vorbis,and arithmetic20ms PCM front segment. No private media,encoder,FFmpeg or network.',cases=[dict(file='one.ogg',cap=6,to_us=10000,frames=960),dict(file='tail.ogg',cap=6,to_us=10000,frames=1920),dict(file='vorbis.ogg',cap=6,to_us=1000,frames=411)],sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
