#!/usr/bin/env python3
"""Original PS common-grid oracles and mixed-resolution synthetic videos.

Fraction/Decimal apply saved normative protocol weights independently of Rust.
Encoded payloads start from authored absolute native targets. No private
media, foreign decoder, FFmpeg, network or ordinary test-time generation.
"""
from decimal import Decimal as D, localcontext
from fractions import Fraction as F
import hashlib,json
from generate_aac_ps_history_fixtures import COARSE,FINE,ICC,targets,zero,encode
from generate_aac_ps_fixtures import sbr
from generate_he_aac_packet_fixtures import DEST,field,packet,asc,video_fixture

PROTOCOL=json.loads((DEST/'aac-ps-mapping-protocol.json').read_text())

def map_exact(values,bands):
    source=len(values)
    if source==10:
        values=[v for v in values for _ in range(2)];source=20
    if source==bands:return list(values)
    assert (source,bands) in [(20,34),(34,20)]
    rows=PROTOCOL['up20_to34' if source==20 else 'down34_to20']
    return [sum((F(values[i])*w for i,w in row['terms']),F(0))/row['denominator'] for row in rows]

def map_int(values,bands):return [int(v) for v in map_exact(values,bands)]

def map_phase(values,bands):
    native={5:10,11:20,17:34}[len(values)]
    output=map_int(list(values)+[0]*(native-len(values)),bands)
    phase=11 if bands==20 else 17
    return output[:phase]+[0]*(bands-phase)

def physical(value,phase_oracle):
    iid_grid=FINE if value['iid_mode']>=3 else COARSE;offset=len(iid_grid)//2
    return dict(iid_db=[iid_grid[i+offset] for i in value['iid']],
                coherence=[ICC[i] for i in value['icc']],
                ipd_radians=[phase_oracle[i] for i in value['ipd']],
                opd_radians=[phase_oracle[i] for i in value['opd']])

def mapped(native,bands,phase_oracle):
    output=dict(bands=bands,iid_mode=native['iid_mode'],icc_mode=native['icc_mode'],
                iid_enabled=native['iid_enabled'],icc_enabled=native['icc_enabled'],phase_enabled=native['phase_enabled'],
                iid=map_int(native['iid'],bands),icc=map_int(native['icc'],bands),
                ipd=map_phase(native['ipd'],bands),opd=map_phase(native['opd'],bands))
    output['levels']=physical(output,phase_oracle)
    return output

def choose(previous,iid_mode,icc_mode):
    if iid_mode is None and icc_mode is None:return previous
    return 34 if any(mode is not None and mode%3==2 for mode in [iid_mode,icc_mode]) else 20

def main():
    phase=[r['value'] for r in json.loads((DEST/'aac-ps-dequant-oracles.json').read_text())['phase']]
    integer=[];real=[];phase_cases=[];native=[]
    for count in [10,20,34]:
        patterns=[[0]*count,[32767]*count,[-32768]*count,
                  [32767 if i%2 else -32768 for i in range(count)],
                  [(i*5)%31-15 for i in range(count)]]
        for value in [7,-7]:
            patterns += [[value if i==b else 0 for i in range(count)] for b in range(count)]
        for bands in [20,34]:
            for values in patterns:integer.append(dict(source=values,bands=bands,expected=map_int(values,bands)))
    with localcontext() as ctx:
        ctx.prec=90
        for count in [20,34]:
            patterns=[[F(0)]*count,[F(3,8)]*count,
                      [F((i*13)%23-11,8) for i in range(count)]]
            patterns += [[F(5,4) if i==b else F(0) for i in range(count)] for b in range(count)]
            for bands in [20,34]:
                for values in patterns:
                    output=map_exact(values,bands)
                    real.append(dict(source=[str(D(v.numerator)/D(v.denominator)) for v in values],bands=bands,
                        expected=[str(D(v.numerator)/D(v.denominator)) for v in output]))
    for count in [5,11,17]:
        patterns=[[0]*count,[7]*count,[7 if i%2==0 else 0 for i in range(count)],[(i*3+1)%8 for i in range(count)]]
        for bands in [20,34]:
            for values in patterns:phase_cases.append(dict(source=values,bands=bands,expected=map_phase(values,bands)))
    for im in range(6):
        for cm in range(6):
            for ep in [False,True]:
                value=targets(im,cm,im*3+cm,phase=ep)
                for bands in [20,34]:native.append(dict(source=value,expected=mapped(value,bands,phase)))
    history=json.loads((DEST/'aac-ps-history-oracles.json').read_text());sequences=[]
    for sequence in history['sequences']:
        previous=20;frames=[]
        for record in sequence['frames']:
            value=record['expected'];header=value['header'];bands=choose(previous,header['iid_mode'],header['icc_mode'])
            frames.append(dict(previous_bands=previous,bands=bands,initialized=value['initialized'],phase_enabled=value['phase_enabled'],
                borders=value['borders'],envelopes=[mapped(v,bands,phase) for v in value['envelopes']]))
            previous=bands
        sequences.append(dict(name=sequence['name'],frames=frames))
    # Every native IID/ICC resolution pair, each with coarse/Ra and fine/Rb.
    # Groups include common-grid changes; headers independently code each
    # packet, while following envelopes alternate coding directions per tool.
    groups=[[(0,0),(1,2),(1,1)],[(2,0),(0,2),(0,1)],[(1,0),(2,1),(2,2)]]
    videos=[];blob=bytearray()
    def save_video(name,controls):
        previous_native=zero();previous_bands=20;frames=[];payloads=[];expected=[]
        for stage,(im,cm,ei,ec,count) in enumerate(controls):
            rows=[targets(im,cm,stage*7+e,ei,ec,ei) for e in range(count)]
            if im%3==1 and cm%3==2:
                rows[0]['iid'][:2]=[-1,-2]
                rows[0]['ipd'][:2]=[7,0];rows[0]['opd'][:2]=[0,7]
            text,borders=encode(True,im,cm,ei,ec,ei,rows,previous_native,32)
            raw=sbr(text,stage);data=packet(raw)
            frames.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data);payloads.append(raw.hex())
            bands=choose(previous_bands,im if ei else None,cm if ec else None)
            expected.append(dict(previous_bands=previous_bands,bands=bands,initialized=True,phase_enabled=ei,borders=borders,
                envelopes=[mapped(v,bands,phase) for v in rows]))
            previous_bands=bands;previous_native=rows[-1]
        case=dict(slots=16,bands=64,frames=frames,asc=asc(24000,48000,16,'explicit',ps=True).hex(),pcm_offset=0,samples=12288)
        video=video_fixture([case],blob,channels=2,filename=name)
        video.pop('pcm_offset');video.pop('samples');video['pcm_acceptance']='pending owned PS synthesis'
        videos.append(dict(video=video,sbr_payloads=payloads,packet_frames=frames,expected=expected))
    for fine in [False,True]:
        for group,pairs in enumerate(groups):
            shift=3 if fine else 0
            save_video(f'he-aac-ps-mixed-{group}-{int(fine)}-synthetic.mp4',[(im+shift,cm+shift,True,True,2) for im,cm in pairs])
    # A stale disabled IID mode=34 must not override an enabled ICC mode=10.
    # Once both tools are disabled, keep the preceding 20-band configuration.
    save_video('he-aac-ps-disabled-selection-synthetic.mp4',[(2,2,True,True,1),(2,0,False,True,1),(2,0,False,False,1)])
    (DEST/'he-aac-ps-mapping-packets.bin').write_bytes(blob)
    oracle=dict(kind='original PS common mapping outputs from Fraction and Decimal over saved normative weights',
                integers=integer,coefficients=real,phases=phase_cases,native=native,sequences=sequences,
                videos=videos,packet_sha256=hashlib.sha256(blob).hexdigest(),protocol_sha256=hashlib.sha256((DEST/'aac-ps-mapping-protocol.json').read_bytes()).hexdigest())
    (DEST/'aac-ps-mapping-oracles.json').write_text(json.dumps(oracle,separators=(',',':'))+'\n')
    print(len(integer),'integer cases;',len(real),'real coefficient cases;',len(native),'native parameter cases;',len(videos),'videos')

if __name__=='__main__':main()
