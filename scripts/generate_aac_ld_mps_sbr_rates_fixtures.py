#!/usr/bin/env python3
"""Own nominal high-rate LD-SBR -> LD-MPS PCM fixtures. Explicit/offline only."""
import json, math, struct
from pathlib import Path
from generate_aac_eld_fixtures import generate as core, mkv
from generate_aac_eld_cldfb_fixtures import window, synthesis
from generate_aac_eld_sbr_fixtures import analysis, reconstruct, sbr_bits
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_ld_layout_fixtures import LAYOUTS
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_ld_mps_stp_fixtures import config, body
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope, length
from generate_he_aac_packet_fixtures import field, frequency, packed, video_fixture
ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT / 'tests/fixtures/playback-errors'
OUT = FIX / 'aac-ld-mps-sbr-rates'
FRAMES = 16

def main():
    OUT.mkdir(exist_ok=True)
    parents = json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases']
    configs = (FIX/'aac-ld-mps-assemble/configs.bin').read_bytes()
    noise_wire = (FIX/'aac-sbr-noise-protocol.f64le').read_bytes()
    noise = [complex(*struct.unpack_from('<dd', noise_wire, i*16)) for i in range(512)]
    blob, controls, cases = bytearray(), [], []
    def rows(frames):
        out = []
        for bits in frames:
            wire = packed(bits)
            out.append(dict(offset=len(blob), bytes=len(wire), core_bits=len(bits)))
            blob.extend(wire)
        return out
    _, hb, lb, _ = tables(5, 10, 0, False, 0, 0)
    h = header(0, True)
    h = h[:1] + field(0,4) + field(14,4) + h[9:]
    for n in [480,512]:
        for cfg in [1,2,6]:
            files = core(True, specs=[(n,48000,cfg,'base',0,0,False)], frame_count=FRAMES)
            source = json.loads(files['manifest.json'])['cases'][0]
            pcm_wire = files[source['name']+'.f32le']
            pcm = struct.unpack('<'+'f'*(len(pcm_wire)//4), pcm_wire)
            elements, mapping, _ = LAYOUTS[cfg]
            channels = len(mapping)
            lfe = mapping[sum(2 if e==1 else 1 for e in elements[:elements.index(3)])] if 3 in elements else None
            qmf = []
            for ch in range(channels):
                low = analysis(pcm[ch::channels],window(),32)
                qmf.append([r+[0j]*32 for r in low] if ch==lfe else reconstruct([r[:5] for r in low],n//32,noise,geometry=(5,10,[1,2,3,None,None])))
            for double in [False,True]:
                rate, sn = 48000*(1+double), n*(1+double)
                bins = 128 if double else 64
                group = bins//math.gcd(bins,sn)
                slots = group*sn//bins
                assert FRAMES%group==0
                planar = [[v/(32768 if double else 65536) for v in synthesis(r,64 if double else 32,window())] for r in qmf]
                golden = b''.join(struct.pack('<'+'f'*channels,*(p[t] for p in planar)) for t in range(len(planar[0])))
                prefix = field(31,5)+field(7,6)+frequency(48000)+field(cfg,4)+field(n==480,1)+'0001'+field(double,1)+'1'+h*sum(e!=3 for e in elements)
                asc = packed(prefix+'000000')
                frames = [r['bits']+''.join(sbr_bits(2 if e==1 else 1,len(hb)-1,len(lb)-1,n//32,i,True) for e in elements if e!=3) for i,r in enumerate(source['frames'])]
                name = f'n{n}-r48000-c{cfg}-x{1+double}'
                base = dict(name=name,n=n,profile=39,factor=1,configuration=cfg,channels=channels,input_channels=channels,rate=rate,source_rate=48000,container_rate=rate,container_frame_samples=sn,samples=FRAMES*sn,pcm_offset=0,asc=asc.hex(),frames=rows(frames),out_n=sn,slots=slots,bands=bins,video=name+'-control.mp4',golden=name+'-control.f32le')
                (OUT/base['video']).write_bytes(video_fixture([base],blob,channels,write=False)['data'])
                (OUT/base['golden']).write_bytes(golden)
                controls.append(base)
                for tree in ([2,3,4] if cfg==2 else [0,1,7] if cfg==1 else [5,6]):
                    c = next(c for c in parents if c['tree']==tree and c['variant']==1 and c['bands']==23)
                    ssc = config(c,48000,slots,configs[c['config_offset']:c['config_offset']+c['config_bytes']],0)
                    ssc = packed(frequency(rate)+raw(ssc)[4:])
                    asc = packed(prefix+field(2,4)+length(len(ssc))+raw(ssc)+'000000')
                    suffix, reference = ['']*FRAMES, []
                    for i in range(FRAMES//group):
                        payload,_ = body(c,i,slots,'alternating',False)
                        suffix[(i+1)*group-1] = envelope(9,1 if i==0 else 0,3,(bytes([len(ssc)])+ssc if i==0 else b'')+payload,True)
                        reference.append(payload.hex())
                    packets = [b+t for b,t in zip(frames,suffix)]
                    stem = name+f'-tree{tree}'
                    outputs = [6,6,6,8,8,8,8,2][tree]
                    case = dict(base,name=stem,asc=asc.hex(),frames=rows(packets),tree=tree,channels=outputs,group=group,ssc=ssc.hex(),reference=reference,core_gold=base['golden'],video=stem+'.mp4',file=stem+'.mkv',live_prefix_packets=group+2,edges=[group*sn,2*group*sn,3*group*sn])
                    (OUT/case['video']).write_bytes(video_fixture([case],blob,outputs,write=False)['data'])
                    (OUT/case['file']).write_bytes(mkv(asc,[packed(b) for b in packets],rate,sn,outputs))
                    cases.append(case)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,exports=[dict(c,video=c['file'],name=c['name']+'-matroska') for c in cases],scope='Own nominal48kHz ELD, single/dual SBR into64/128-band MPS PCM. Component composition,not whole-codec conformance.'),separators=(',',':'))+'\n')
    print(len(cases),'programmes;',len(controls),'independent PCM controls')
if __name__ == '__main__':
    main()
