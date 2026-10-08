#!/usr/bin/env python3
"""Offline Decimal/Fraction PS matrix-controller and hybrid-mixing references."""
import json,hashlib,struct
from decimal import Decimal as D
from generate_aac_ps_mixing_oracles import matrix
from generate_aac_ps_phase_history_fixtures import trace as phase_trace
from generate_aac_ps_mapping_fixtures import mapped,choose,PROTOCOL
from generate_aac_ps_interpolation_fixtures import interpolate
from generate_aac_ps_history_fixtures import targets,zero,encode
from generate_aac_ps_fixtures import sbr
from generate_he_aac_packet_fixtures import DEST,packet,asc,video_fixture

class Store:
    def __init__(self):self.data=bytearray();self.saved={}
    def put(self,values):
        raw=b''.join(struct.pack('<d',float(v)) for v in values)
        if raw not in self.saved:self.saved[raw]=len(self.data);self.data.extend(raw)
        return [self.saved[raw],len(values)]
def real_values(envelope):
    return [matrix(str(db),D(coherence),'b' if envelope['icc_mode']>=3 else 'a') for db,coherence in zip(envelope['levels']['iid_db'],envelope['levels']['coherence'])]
def unrotated(real):return [[[h,D(0)] for h in row] for row in real]
def map_real(real,bands):
    if len(real)==bands:return real
    rows=PROTOCOL['up20_to34' if len(real)==20 else 'down34_to20']
    return [[sum((real[i][c]*w for i,w in row['terms']),D(0))/row['denominator'] for c in range(4)] for row in rows]
def flatten(matrix):return [v for row in matrix for pair in row for v in pair]
def signals(n,k):
    return ((D((13*n+7*k)%23-11)/16,D((3*n+11*k)%31-15)/32),
            (D((17*n+5*k)%29-14)/32,D((19*n+3*k)%37-18)/16))
def mul(a,b):return a[0]*b[0]-a[1]*b[1],a[0]*b[1]+a[1]*b[0]
def hybrid_outputs(coefficients,bands):
    bindings=PROTOCOL['hybrid20' if bands==20 else 'hybrid34'];output=[[],[]]
    for n,slot in enumerate(coefficients):
        stereo=[[],[]]
        for k,binding in enumerate(bindings):
            h=slot[binding['parameter']]
            if binding['conjugate']:h=[(re,-im) for re,im in h]
            source,diffuse=signals(n,k)
            for channel,(direct,decor) in enumerate([(0,2),(1,3)]):
                a=mul(h[direct],source);b=mul(h[decor],diffuse)
                stereo[channel].extend([a[0]+b[0],a[1]+b[1]])
        for c in range(2):output[c].append(stereo[c])
    return output

def timeline(common,phase,slots,store,mix=False):
    bands=20;real=[[D(0)]*4 for _ in range(20)];previous=unrotated(real);frames=[]
    for parameters,phased in zip(common,phase):
        accepted=phased['accepted'];changed=parameters['bands']!=bands;coefficients=[]
        if accepted:
            if changed:
                bands=parameters['bands'];real=map_real(real,bands);previous=unrotated(real)
            if not parameters['envelopes'] and not parameters['phase_enabled']:previous=unrotated(real)
            endpoints=[[[[D(re),D(im)] for re,im in row] for row in envelope] for envelope in phased['endpoints']]
            coefficients=interpolate(previous,endpoints,parameters['borders'],slots)
            previous=coefficients[-1]
            if parameters['envelopes']:real=real_values(parameters['envelopes'][-1])
        frame=dict(accepted=accepted,bands=bands,bands_changed=changed if accepted else False,slots=slots,
                   real=store.put([v for row in real for v in row]),last=store.put(flatten(previous)),
                   coefficients=[store.put(flatten(row)) for row in coefficients],phase=phased['history'])
        if accepted and mix:frame['output']=[[store.put(row) for row in channel] for channel in hybrid_outputs(coefficients,bands)]
        frames.append(frame)
    return frames

def main():
    store=Store();common=json.loads((DEST/'aac-ps-mapping-oracles.json').read_text());phase=json.loads((DEST/'aac-ps-phase-history-oracles.json').read_text());native=json.loads((DEST/'aac-ps-history-oracles.json').read_text())
    sequences=[dict(name=c['name'],frames=timeline(c['frames'],p['frames'],n['slots'],store)) for c,p,n in zip(common['sequences'],phase['sequences'],native['sequences'])]
    existing=[]
    for source,values,phases in [('aac-ps-mapping-oracles.json',common['videos'],phase['mapped_videos']),('aac-ps-phase-history-oracles.json',phase['videos'],phase['videos'])]:
        for video,p in zip(values,phases):
            c=video['expected'] if source=='aac-ps-mapping-oracles.json' else video['common']
            ph=p['frames'] if source=='aac-ps-mapping-oracles.json' else p['expected']
            existing.append(dict(source=source,file=video['video']['file'],frames=timeline(c,ph,32,store,True)))
    blob=bytearray();videos=[]
    phase_grid=[r['value'] for r in json.loads((DEST/'aac-ps-dequant-oracles.json').read_text())['phase']]
    configs=[('matrix-retain',[(1,2,True),(1,0,False),(1,0,True)]),('matrix-grid-retain',[(1,2,True),(5,0,True),(1,0,False)])]
    for name,controls in configs:
        before=zero();previous_bands=20;frames=[];payloads=[];packets=[]
        for stage,(mode,count,enabled) in enumerate(controls):
            rows=[targets(mode,mode,stage*7+e,phase=enabled) for e in range(count)]
            text,borders=encode(True,mode,mode,True,True,enabled,rows,before,32)
            raw=sbr(text,stage);data=packet(raw);packets.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data);payloads.append(raw.hex());bands=choose(previous_bands,mode,mode)
            frames.append(dict(previous_bands=previous_bands,bands=bands,initialized=True,phase_enabled=enabled,borders=borders,envelopes=[mapped(v,bands,phase_grid) for v in rows]));previous_bands=bands
            if rows:before=rows[-1]
        case=dict(slots=16,bands=64,frames=packets,asc=asc(24000,48000,16,'explicit',ps=True).hex(),pcm_offset=0,samples=12288)
        video=video_fixture([case],blob,channels=2,filename=f'he-aac-ps-{name}-synthetic.mp4');video.pop('pcm_offset');video.pop('samples');video['pcm_acceptance']='pending owned PS hybrid/decorrelation/synthesis'
        videos.append(dict(video=video,sbr_payloads=payloads,packet_frames=packets,expected=timeline(frames,phase_trace(frames),32,store,True)))
    (DEST/'he-aac-ps-matrix-controller-packets.bin').write_bytes(blob)
    (DEST/'aac-ps-matrix-controller-coefficients.bin').write_bytes(store.data)
    (DEST/'aac-ps-matrix-controller-oracles.json').write_text(json.dumps(dict(kind='independent Decimal endpoints, real-grid weights, Fraction timing and original rational hybrid signals; binary64 LE snapshots',sequences=sequences,existing_videos=existing,videos=videos,packet_sha256=hashlib.sha256(blob).hexdigest(),coefficient_sha256=hashlib.sha256(store.data).hexdigest()),separators=(',',':'))+'\n')
    print(len(sequences),'sequences;',len(existing),'existing videos;',len(videos),'new videos;',len(store.data),'reference bytes')
if __name__=='__main__':main()
