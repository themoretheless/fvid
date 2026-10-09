#!/usr/bin/env python3
"""Original SSR core windows plus existing authored PS; independent stereo gold."""
import json
import struct
from generate_aac_ssr_coupling_fixtures import silent
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture


def main():
    video = json.loads((DEST/'aac-ps-matrix-controller-oracles.json').read_text())['videos'][0]
    matrix = json.loads((DEST/'aac-ps-dsp-oracles.json').read_text())
    scalar = (DEST/'aac-ps-dsp-reference.bin').read_bytes()
    ref = next(c for c in matrix['cases'] if c['source']['kind']=='sbr-video'
               and c['source']['name']==video['video']['file'] and c['zero_eof'])
    references = {}
    for rate,key in [(24000,'Core'),(48000,'Double')]:
        pcm=[]
        for row in ref['frames']:
            channels=[]
            for offset,count in row[key]:
                channels.append([v[0]/32768. for v in struct.iter_unpack('<d',scalar[offset:offset+count*8])])
            pcm += [v for pair in zip(*channels) for v in pair]
        filename='aac-ssr-ps-'+str(rate)+'-reference.f64le'
        (DEST/filename).write_bytes(struct.pack('<'+str(len(pcm))+'d',*pcm))
        references[rate]=filename
    blob=bytearray();cases=[];controls=[]
    for name,sequences in [('long',[0,0,0]),('start-stop',[0,1,3]),('start-short-stop',[1,2,3])]:
        core_frames=[];ps_frames=[]
        for i,seq in enumerate(sequences):
            core='0000000'+silent(seq,i%2,0,False,False)
            packet=packed(core+'111')
            core_frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet)
            payload=bytes.fromhex(video['sbr_payloads'][i]);n=len(payload)
            fill='110'+(field(n,4) if n<15 else '1111'+field(n-14,8))+''.join(field(b,8) for b in payload)
            packet=packed(core+fill+'111')
            ps_frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet)
        control=dict(name=name,asc=packed(field(3,5)+frequency(24000)+'0001'+'000').hex(),frames=core_frames,
                     slots=16,bands=32,pcm_offset=0,container_rate=24000,container_frame_samples=1024,samples=3072)
        control['video']=video_fixture([control],blob,filename='aac-ssr-ps-'+name+'-core-control-synthetic.mp4')
        controls.append(control)
        for signal in ('explicit','sync'):
            for rate in (24000,48000):
                config=(field(29,5)+frequency(24000)+'0001'+frequency(rate)+field(3,5)+'000' if signal=='explicit'
                        else field(3,5)+frequency(24000)+'0001'+'000'+field(0x2b7,11)+field(5,5)+'1'+frequency(rate)+field(0x548,11)+'1')
                case=dict(name=name+'-'+signal+'-'+str(rate),asc=packed(config).hex(),frames=ps_frames,
                          slots=16,bands=rate//750,pcm_offset=0,container_rate=rate,
                          container_frame_samples=rate//24000*1024,samples=rate//24000*3072,
                          reference=references[rate])
                case['video']=video_fixture([case],blob,channels=2,filename='aac-ssr-ps-'+case['name']+'-synthetic.mp4')
                cases.append(case)
    (DEST/'aac-ssr-ps-packets.bin').write_bytes(blob)
    (DEST/'aac-ssr-ps.json').write_text(json.dumps(dict(controls=controls,cases=cases,payloads=video['sbr_payloads'],
        error='AAC SSR parametric stereo synthesis is not implemented',
        qualification='Core and independent PS stage acceptance; native combined SSR/PS remains unsupported.',
        provenance='Own silent SSR sine/KBD core across three valid window schedules; own authored SBR/PS matrix payloads and independent Decimal PS/direct QMF scalar reference. No private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
