"""Authored PCM setup transitions and signed composition origins; no encoders."""
from pathlib import Path
import hashlib, json, struct
from generate_mp4_audio_reconfiguration import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom, word
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-reconfigured-seek-endpoint'
def main():
    files, cases = {}, []
    for profile in ['changing', 'constant']:
        shapes = [(b'sowt',8000,1,16),(b'fl32',8000,1,32),(b'in24',8000,1,24)] if profile == 'changing' else [(b'sowt',8000,1,16)]*3
        own = generate(pcm_shapes=shapes, pcm_only=True)
        golden = b''.join(own[f'pcm-{i}.f32le'] for i in range(6))
        files[profile+'.f32le'] = golden
        for container, source in [('regular','pcm.mp4'),('fragmented','pcm-fragmented.mp4')]:
            for shift in [-10000, 0, 10000, 200000]:
                def rewrite(data, audio=False):
                    out = b''
                    for k,b in atoms(data):
                        selected = audio
                        if k == b'trak': selected = dict(atoms(dict(atoms(b))[b'mdia']))[b'hdlr'][8:12] == b'soun'
                        if k == b'traf': selected = struct.unpack_from('>I',dict(atoms(b))[b'tfhd'],4)[0] == 2
                        if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl',b'moof',b'traf']:
                            b = rewrite(b, selected)
                            if k == b'stbl' and audio and container == 'regular':
                                b += atom(b'ctts',word(1<<24,1,6)+struct.pack('>i',shift))
                        elif k == b'trun' and audio:
                            flags,count,offset = struct.unpack('>IIi',b)
                            assert count == 1 and flags == 1
                            b = word((1<<24)|0x801,1)+struct.pack('>ii',offset+4,shift)
                        elif k == b'mvhd': b=b[:12]+word(1000,max(120,120+shift//1000))+b[20:]
                        elif k == b'tkhd': b=b[:20]+word(max(0,120+shift//1000) if audio else 120)+b[24:]
                        out += atom(k,b)
                    return out
                name=f'{profile}-{container}-{shift}.mp4'
                files[name]=rewrite(own[source])
                cases.append(dict(file=name,profile=profile,container=container,shift_ticks=shift,scale=1000000,end_ticks=120000+shift,oracle=profile+'.f32le'))
    from generate_mp4_aac_quicktime_regressions import generate as aac_generate
    aac = aac_generate()
    files['aac-control.mp4'] = aac['iso-v0-direct.mp4']
    files['aac-reference.f32le'] = aac['expected.f32le']
    aac_cases = []
    for shift in [-480,0,480,4800]:
        def aac_shift(data,audio=False):
            out=b''
            for k,b in atoms(data):
                selected=audio
                if k==b'trak': selected=dict(atoms(dict(atoms(b))[b'mdia']))[b'hdlr'][8:12]==b'soun'
                if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                    b=aac_shift(b,selected)
                    if k==b'stbl' and audio: b+=atom(b'ctts',word(1<<24,1,3)+struct.pack('>i',shift))
                elif k==b'mvhd': b=b[:16]+word(max(3072,3072+shift))+b[20:]
                elif k==b'tkhd' and audio: b=b[:20]+word(3072+shift)+b[24:]
                out+=atom(k,b)
            return out
        name=f'aac-{shift}.mp4';files[name]=aac_shift(aac['iso-v0-direct.mp4']);aac_cases.append(dict(file=name,shift_ticks=shift))
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in files.items(): (OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Authored 120ms 16x16 VP9 + six nonzero 8kHz mono PCM packets, three selected equivalent or changing precision setups; signed CTTS/TRUN origins with exact integral media clocks. Plus authored 48kHz mono AAC-LC pulse/overlap with analytical IMDCT reference. No external encoder, private media/parameters, FFmpeg or network.','cases':cases,'aac_cases':aac_cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__ == '__main__': main()
