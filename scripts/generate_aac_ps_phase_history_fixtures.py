#!/usr/bin/env python3
"""Original phase-history traces and videos; offline Decimal numeric oracle."""
import json,hashlib
from decimal import Decimal as D
from generate_aac_ps_mixing_oracles import matrix,phase_coefficients
from generate_aac_ps_mapping_fixtures import mapped,choose
from generate_aac_ps_history_fixtures import targets,zero,encode
from generate_aac_ps_fixtures import sbr
from generate_he_aac_packet_fixtures import DEST,packet,asc,video_fixture

def trace(frames):
    bands=20;ipd=[[0]*20 for _ in range(2)];opd=[[0]*20 for _ in range(2)];result=[]
    for frame in frames:
        accepted=frame['initialized'];changed=frame['bands']!=bands;endpoints=[]
        if accepted:
            if changed:
                bands=frame['bands'];ipd=[[0]*bands for _ in range(2)];opd=[[0]*bands for _ in range(2)]
            for envelope in frame['envelopes']:
                values=[]
                for b in range(bands):
                    real=matrix(str(envelope['levels']['iid_db'][b]),D(envelope['levels']['coherence'][b]),'b' if envelope['icc_mode']>=3 else 'a')
                    pi=[ipd[0][b],ipd[1][b],envelope['ipd'][b]] if envelope['phase_enabled'] else [0]*3
                    po=[opd[0][b],opd[1][b],envelope['opd'][b]] if envelope['phase_enabled'] else [0]*3
                    values.append([[str(re),str(im)] for re,im in phase_coefficients(real,pi,po)])
                endpoints.append(values)
                ipd=[ipd[1],list(envelope['ipd'])];opd=[opd[1],list(envelope['opd'])]
        result.append(dict(accepted=accepted,bands=bands,bands_changed=changed if accepted else False,endpoints=endpoints,history=dict(ipd=[list(v) for v in ipd],opd=[list(v) for v in opd])))
    return result

def main():
    common=json.loads((DEST/'aac-ps-mapping-oracles.json').read_text())
    sequences=[dict(name=s['name'],frames=trace(s['frames'])) for s in common['sequences']]
    mapped_videos=[dict(file=v['video']['file'],frames=trace(v['expected'])) for v in common['videos']]
    phase=[r['value'] for r in json.loads((DEST/'aac-ps-dequant-oracles.json').read_text())['phase']]
    blob=bytearray();videos=[]
    configs=[('phase-history',[(1,2,True),(1,0,True),(1,2,True)]),('phase-disable',[(5,2,True),(5,1,False),(5,2,True)]),('phase-grid-transition',[(1,2,True),(2,0,True),(2,2,True)]),('phase-startup-34',[(2,0,True),(2,2,True),(2,2,True)])]
    for name,controls in configs:
        native_previous=zero();previous_bands=20;payloads=[];packets=[];frames=[]
        for stage,(mode,count,enabled) in enumerate(controls):
            rows=[targets(mode,mode,stage*7+e,phase=enabled) for e in range(count)]
            text,borders=encode(True,mode,mode,True,True,enabled,rows,native_previous,32)
            raw=sbr(text,stage);data=packet(raw);packets.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data);payloads.append(raw.hex())
            bands=choose(previous_bands,mode,mode)
            frames.append(dict(previous_bands=previous_bands,bands=bands,initialized=not(name=='phase-startup-34' and stage==0),phase_enabled=enabled,borders=borders,envelopes=[mapped(v,bands,phase) for v in rows]))
            previous_bands=bands
            if rows:native_previous=rows[-1]
        case=dict(slots=16,bands=64,frames=packets,asc=asc(24000,48000,16,'explicit',ps=True).hex(),pcm_offset=0,samples=12288)
        video=video_fixture([case],blob,channels=2,filename=f'he-aac-ps-{name}-synthetic.mp4')
        video.pop('pcm_offset');video.pop('samples');video['pcm_acceptance']='pending owned PS hybrid/decorrelation/synthesis'
        videos.append(dict(video=video,sbr_payloads=payloads,packet_frames=packets,common=frames,expected=trace(frames)))
    (DEST/'he-aac-ps-phase-history-packets.bin').write_bytes(blob)
    (DEST/'aac-ps-phase-history-oracles.json').write_text(json.dumps(dict(kind='original absolute phase position traces with independent Decimal rotations',sequences=sequences,mapped_videos=mapped_videos,videos=videos,packet_sha256=hashlib.sha256(blob).hexdigest()),separators=(',',':'))+'\n')
    print(len(sequences),'sequences;',len(mapped_videos),'existing mixed videos;',len(videos),'new videos')
if __name__=='__main__':main()
