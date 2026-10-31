#!/usr/bin/env python3
"""Own Q31 direct-PCM and temporal-history acceptance, offline generation only."""
import json, struct
from pathlib import Path
from generate_aac_eld_fixtures import generate, mkv
from generate_aac_eld_sbr_fixtures import sbr_bits, analysis, reconstruct
from generate_aac_eld_cldfb_fixtures import window, synthesis
from generate_aac_sbr_frequency_oracles import tables
from generate_he_aac_packet_fixtures import packed, video_fixture
ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT/'tests/fixtures/playback-errors'
OUT = FIX/'aac-ld-sbr-q31'


def main():
    OUT.mkdir(exist_ok=True)
    files = generate(True, [(n,24000,2,'base',0,0,False) for n in [480,512]], frame_count=8)
    cores = json.loads(files['manifest.json'])['cases']
    parent = FIX/'aac-eld-sbr'
    models = json.loads((parent/'manifest.json').read_text())['cases']
    saved = (parent/'packets.bin').read_bytes()
    raw_noise = (FIX/'aac-sbr-noise-protocol.f64le').read_bytes()
    noise = [complex(*struct.unpack_from('<dd',raw_noise,i*16)) for i in range(512)]
    c = window()
    _, high, low, _ = tables(10,27,0,False,0,0)
    blob, cases, errors = bytearray(), [], []
    for core in cores:
        n = core['n']
        pcm = list(struct.unpack('<'+'f'*(len(files[core['name']+'.f32le'])//4),files[core['name']+'.f32le']))
        lows = [analysis(pcm[ch::2], c) for ch in range(2)]
        for mode in ['invf','coupled','coupled-pan']:
            model = next(v for v in models if v['name'] == f'n{n}-c2-{mode}')
            for level in [7,30,31,32]:
                name = f'n{n}-{mode}-q{level}'
                frames, packets = [], []
                levels = [7]*8
                levels[3] = min(level,31)
                if level == 32:levels[4] = 32
                for i, source in enumerate(core['frames']):
                    delta = (1 if level==32 else 7-level) if level!=7 and i==4 else 0 if level!=7 and i==5 else None
                    bits = source['bits']+sbr_bits(2,len(high)-1,len(low)-1,n//32,i,True,mode,noise_level=levels[i],noise_delta=delta)
                    wire = packed(bits)
                    if level==7:
                        f=model['frames'][i];assert wire==saved[f['offset']:f['offset']+f['bytes']]
                    frames.append(dict(offset=len(blob),bytes=len(wire)));blob.extend(wire);packets.append(wire)
                case = dict(model,name=name,frames=frames,samples=8*n*2,mode=mode,level=level,
                            video=name+'.mp4',file=name+'.mkv',golden=name+'.f32le',
                            live_prefix_packets=2,edges=[2*n,6*n,8*n,10*n])
                (OUT/case['video']).write_bytes(video_fixture([case],blob,2,write=False)['data'])
                (OUT/case['file']).write_bytes(mkv(bytes.fromhex(case['asc']),packets,48000,n*2,2))
                if level==32:
                    case.update(bad_frame=4,error='SBR noise level exceeds normative range: 32')
                    errors.append(case)
                else:
                    rows = [reconstruct(v,n//32,noise,mode,ch,noise_levels=levels) for ch,v in enumerate(lows)]
                    planar = [[v/32768 for v in synthesis(r,64,c)] for r in rows]
                    gold = b''.join(struct.pack('<ff',a,b) for a,b in zip(*planar))
                    if level==7:assert gold==(parent/(model['name']+'.f32le')).read_bytes()[:len(gold)]
                    (OUT/case['golden']).write_bytes(gold)
                    cases.append(case)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,errors=errors,
        exports=[dict(v,video=v['file'],name=v['name']+'-matroska') for v in cases],
        scope='Own480/512 stereo LD-SBR/CRC: inverse filtering,centered/asymmetric coupling. Q7/Q30/Q31 absolute plus temporal return toQ7; independent direct CLDFB/HF/noise PCM. Q32 remains refusal. No private/ISO media/parameter sets,production decoder,FFmpeg,network or subprocess.'),separators=(',',':'))+'\n')
    print(len(cases),'acceptance programmes;',len(errors),'Q32 counterexamples; both containers')


if __name__=='__main__':main()
