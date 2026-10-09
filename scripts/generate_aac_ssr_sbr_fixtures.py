#!/usr/bin/env python3
"""Authored SSR window transitions plus SBR; no external codec or private media."""
import json,math,re,struct
from decimal import Decimal as D
from generate_aac_sbr_dsp_fixtures import synthesis
from generate_aac_ssr_fixtures import channel,oracle
from generate_aac_main_sbr_oracle import reference
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def main():
    binary=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    ref=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases'] if c['slots']==16 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    blob=bytearray();rows=[];controls=[]
    for i,seq in enumerate([0,1,2,2,3,0]):
        # SCE0, zero bands, no pulse/TNS/gain. SSR accepts the same ICS shape.
        core='0000000'+field(100,8)+'0'+field(seq,2)+'0'+field(0,4 if seq==2 else 6)+('1111111' if seq==2 else '0')+'000'
        p=packed(core+'111');controls.append(dict(offset=len(blob),bytes=len(p)));blob.extend(p)
        r=ref['frames'][i%3];raw=binary[r['offset']:r['offset']+r['byte_length']];n=len(raw)
        p=packed(core+'110'+(field(n,4) if n<15 else '1111'+field(n-14,8))+''.join(field(b,8) for b in raw)+'111')
        rows.append(dict(offset=len(blob),bytes=len(p)));blob.extend(p)
    cases=[]
    for signal in ('explicit','sync','implicit','core-control'):
        prefix=field(3,5)+frequency(24000)+'0001'+'000'
        config=(field(5,5)+frequency(24000)+'0001'+frequency(48000)+field(3,5)+'000' if signal=='explicit' else prefix+(field(0x2b7,11)+field(5,5)+'1'+frequency(48000) if signal=='sync' else ''))
        c=dict(name=signal,asc=packed(config).hex(),frames=controls if signal=='core-control' else rows,slots=16,bands=64,container_rate=24000 if signal=='core-control' else 48000,container_frame_samples=1024 if signal=='core-control' else 2048,pcm_offset=0,samples=6144 if signal=='core-control' else 12288)
        c['video']=video_fixture([c],blob,filename='aac-ssr-sbr-'+signal+'-synthetic.mp4');cases.append(c)
    source=DEST.parents[2]/'crates/fvid-media/src/owned_aac/aac_sbr_qmf_window.rs'
    window=[float(x) for x in re.findall(r'-?\d+\.\d+',source.read_text().split('= [',1)[1])]
    noise_bytes=(DEST/'aac-sbr-noise-protocol.f64le').read_bytes()
    noise=[complex(*struct.unpack_from('<dd',noise_bytes,i*16)) for i in range(512)]
    amplitude=float((D(128)/D(3)).sqrt()*D('1.584893192'))*0.99999999999999
    high=[[amplitude*noise[(t*17+b+1)%512] for b in range(17)] for t in range(192)]
    pcm=synthesis(high,64,window)
    (DEST/'aac-ssr-sbr-reference.f64le').write_bytes(struct.pack('<'+str(len(pcm))+'d',*pcm))
    active_rows=[];active_controls=[]
    for i,seq in enumerate([0,1,2,2,3,0]):
        core='0000000'+channel(i,seq,0,0,True,False)
        p=packed(core+'111');active_controls.append(dict(offset=len(blob),bytes=len(p)));blob.extend(p)
        r=ref['frames'][i%3];raw=binary[r['offset']:r['offset']+r['byte_length']];n=len(raw)
        p=packed(core+'110'+(field(n,4) if n<15 else '1111'+field(n-14,8))+''.join(field(b,8) for b in raw)+'111')
        active_rows.append(dict(offset=len(blob),bytes=len(p)));blob.extend(p)
    base=next(c for c in cases if c['name']=='core-control')
    active_control=dict(base,frames=active_controls)
    active_control['video']=video_fixture([active_control],blob,filename='aac-ssr-sbr-active-core-control-synthetic.mp4')
    core_pcm,_=oracle([0]*6,1,True)
    (DEST/'aac-ssr-sbr-active-core-reference.f32le').write_bytes(core_pcm)
    scalar=reference(pcm_override=list(struct.unpack('<'+str(len(core_pcm)//4)+'f',core_pcm)))
    reference_file='aac-ssr-sbr-active-reference.f64le'
    (DEST/reference_file).write_bytes(struct.pack('<'+str(len(scalar))+'d',*scalar))
    for base in list(cases):
        if base['name']=='core-control':continue
        c=dict(base,name=base['name']+'-active',frames=active_rows,reference=reference_file)
        c['video']=video_fixture([c],blob,filename='aac-ssr-sbr-'+c['name']+'-synthetic.mp4');cases.append(c)
    (DEST/'aac-ssr-sbr-packets.bin').write_bytes(blob)
    (DEST/'aac-ssr-sbr.json').write_text(json.dumps(dict(cases=cases,active_core_control=active_control,provenance='Own silent and nonzero SSR SCE with active gain control across long/start/short/stop transitions; existing authored SBR syntax and owned AVC/container seeds. No private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
