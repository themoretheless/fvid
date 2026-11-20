#!/usr/bin/env python3
"""Own in-band arbitrary-downmix on/off transitions,explicit offline generation."""
import json
from pathlib import Path
from generate_aac_eld_fixtures import mkv
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_he_aac_packet_fixtures import packed,field,video_fixture
ROOT=Path(__file__).resolve().parents[1]
FIX=ROOT/'tests/fixtures/playback-errors'
OUT=FIX/'aac-ld-mps-arbdmx-switch'

def main():
    OUT.mkdir(exist_ok=True)
    cases,controls,errors,blob=[],[],[],bytearray()
    def rows(bits):
        out=[]
        for b in bits:
            wire=packed(b);out.append(dict(offset=len(blob),bytes=len(wire),core_bits=len(b)));blob.extend(wire)
        return out
    for corpus in ['aac-ld-mps-arbdmx','aac-ld-mps-arbdmx-ttt']:
        parent=FIX/corpus;m=json.loads((parent/'manifest.json').read_text());data=(parent/'packets.bin').read_bytes()
        for c in m['controls']:
            ctl=dict(c,frames=rows([raw(data[r['offset']:r['offset']+r['bytes']])[:r['core_bits']]for r in c['frames']]))
            (OUT/ctl['video']).write_bytes((parent/c['video']).read_bytes())
            (OUT/ctl['golden']).write_bytes((parent/c['golden']).read_bytes());controls.append(ctl)
        for source in m['cases']:
            if source['pattern']!='alternating' or source['tree']>=2 and source['variant'] not in [0,1,2]:continue
            ctl=next(c for c in m['controls']if c['golden']==source['core_gold'])
            prefixes=[raw(data[r['offset']:r['offset']+r['bytes']])[:ctl['frames'][i]['core_bits']]for i,r in enumerate(source['frames'])]
            on=bytes.fromhex(source['ssc']);off=packed(raw(on)[:19]+'0'+raw(on)[20:]);assert raw(on)[19]=='1'
            wire=raw(bytes.fromhex(source['asc']));tail=field(2,4)+length(len(on))+raw(on)+'000000';at=wire.rfind(tail);assert at>=0 and packed(wire[:at]+tail).hex()==source['asc'];head=wire[:at]
            for initial in [False,True]:
                frames,reference,control_bits,actual_bits=[],[],[],[]
                phases=[initial if i//3%2==0 else not initial for i in range(12)]
                configs=[(on if enabled else off).hex()for enabled in phases]
                history=[[0]*source['bands']for _ in range(source['input_channels'])]
                for i,(prefix,row,enabled)in enumerate(zip(prefixes,source['frames'],phases)):
                    if i>0 and phases[i]!=phases[i-1]:history=[[0]*source['bands']for _ in range(source['input_channels'])]
                    held=enabled and i>0 and (i%3==0 or i%3==2)
                    ordinary='010'
                    main=[r for r in row['parameters']if not r['id'].startswith('downmix-gain:')]
                    for r in main:
                        shift,levels={'CLD':(15,31),'ICC':(0,8),'CPC':(20,51)}[r['kind']]
                        ordinary+='1111'+'10001'+pcm([r['values'][ps][b]+shift for b in range(len(r['values'][0]))for ps in range(2)],levels)
                    ordinary+='00'+'1001';gains=[];gain_bits=''
                    for r in row['parameters']:
                        if not r['id'].startswith('downmix-gain:'):continue
                        ch=len(gains)
                        values=([history[ch].copy(),history[ch].copy()]if held else r['values'])if enabled else[[0]*source['bands']for _ in range(2)]
                        if enabled:history[ch]=values[-1].copy()
                        gains.append(dict(r,values=values));gain_bits+='1111'+'10001'+pcm([values[ps][b]+15 for b in range(source['bands'])for ps in range(2)],31)
                    control_body=packed(ordinary+gain_bits);body=packed(ordinary+('0101'*len(gains)if held else gain_bits if enabled else''))
                    reference.append(control_body.hex())
                    header=i in [0,2,3,5,6,8,9,11]
                    ssc=on if enabled else off
                    packet=prefix+envelope(9,1 if header else 0,3,(bytes([len(ssc)])+ssc if header else b'')+body,True)
                    actual_bits.append(packet);control_bits.append(prefix+envelope(9,1 if i==0 else 0,3,(bytes([len(on)])+on if i==0 else b'')+control_body,True))
                    f=rows([packet])[0];f.update(parameters=main+gains,slots=row['slots'],smoothing=row['smoothing'],prefix_bits=len(prefix),body=body.hex(),header=header,enabled=enabled,held=held);frames.append(f)
                name=source['name']+f'-initial{int(initial)}'
                asc=packed(head+field(2,4)+length(len(on))+raw(on if initial else off)+'000000')
                case=dict(source,name=name,asc=asc.hex(),frames=frames,reference=reference,configs=configs,initial=initial,video=name+'.mp4',file=name+'.mkv',control=name+'-continuous.mp4',control_frames=rows(control_bits),control_asc=source['asc'],live_prefix_packets=5,edges=[3*source['out_n'],6*source['out_n'],9*source['out_n']])
                (OUT/case['video']).write_bytes(video_fixture([case],blob,case['channels'],write=False)['data'])
                (OUT/case['file']).write_bytes(mkv(asc,[packed(b)for b in actual_bits],case['rate'],case['out_n'],case['channels']))
                control=dict(case,asc=source['asc'],frames=case['control_frames']);(OUT/case['control']).write_bytes(video_fixture([control],blob,case['channels'],write=False)['data'])
                cases.append(case)
                if source['tree']in[0,2]and source['bands']==23 and 'aac-ld-mps-downscaled-'in source['name']:
                    bad_bits=actual_bits.copy();i=6 if initial else 3;body=bytes.fromhex(frames[i]['body'])[:-4];ssc=on
                    bad_bits[i]=prefixes[i]+envelope(9,1,3,bytes([len(ssc)])+ssc+body,True)
                    bad=dict(case,name=name+'-bad-change',video=name+'-bad-change.mp4',frames=rows(bad_bits),good=name,bad_index=i,error='truncated')
                    (OUT/bad['video']).write_bytes(video_fixture([bad],blob,case['channels'],write=False)['data']);errors.append(bad)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,errors=errors,exports=[dict(c,video=c['file'],name=c['name']+'-matroska')for c in cases],scope='Own in-band gain-mode on/off/on/off and inverse;continuous always-on zero-dB compensation control preserves raw M1 and filter histories. Initial SSC may disable gains. Identical headers occur within phases. Component integration,not whole-codec/profile conformance.'),separators=(',',':'))+'\n')
    print(len(cases),'switch programmes;',len(controls),'bare controls;',len(errors),'late bad headers/frames')
if __name__=='__main__':main()
