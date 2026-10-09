#!/usr/bin/env python3
"""Original SSR core windows plus existing authored PS; independent stereo gold."""
import json
import struct
from generate_aac_ssr_coupling_fixtures import silent
from generate_aac_ps_matroska_fixtures import element, number
from generate_aac_ps_worker_fixtures import edited
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
        adts = bytearray()
        for row in ps_frames:
            packet = blob[row['offset']:row['offset']+row['bytes']]
            header = field(0xfff,12)+'0'+'00'+'1'+field(2,2)+frequency(24000)+'0'+'001'+'0000'+field(len(packet)+7,13)+field(0x7ff,11)+'00'
            adts += packed(header)+packet
        adts_name = 'aac-ssr-ps-'+name+'-synthetic.aac'
        (DEST/adts_name).write_bytes(adts)
        control['adts'] = adts_name
        for signal in ('explicit','sync'):
            for rate in (24000,48000):
                config=(field(29,5)+frequency(24000)+'0001'+frequency(rate)+field(3,5)+'000' if signal=='explicit'
                        else field(3,5)+frequency(24000)+'0001'+'000'+field(0x2b7,11)+field(5,5)+'1'+frequency(rate)+field(0x548,11)+'1')
                case=dict(name=name+'-'+signal+'-'+str(rate),asc=packed(config).hex(),frames=ps_frames,
                          slots=16,bands=rate//750,pcm_offset=0,container_rate=rate,
                          container_frame_samples=rate//24000*1024,samples=rate//24000*3072,
                          reference=references[rate])
                case['video']=video_fixture([case],blob,channels=2,filename='aac-ssr-ps-'+case['name']+'-synthetic.mp4')
                audio = element(0xe1,element(0xb5,struct.pack('>d',rate))+number(0x9f,2))
                track = element(0xae,number(0xd7,1)+number(0x73c5,1)+number(0x83,2)+element(0x86,b'A_AAC')+element(0x63a2,packed(config))+audio)
                clusters = bytearray()
                for i,row in enumerate(ps_frames):
                    packet = blob[row['offset']:row['offset']+row['bytes']]
                    ns = (i*case['container_frame_samples']*1_000_000_000+rate//2)//rate
                    clusters += element(0x1f43b675,number(0xe7,ns)+element(0xa3,b'\x81\x00\x00\x80'+packet))
                header = element(0x1a45dfa3,element(0x4282,b'matroska')+number(0x4287,4)+number(0x4285,2))
                info = element(0x1549a966,number(0x2ad7b1,1))
                mka = header+element(0x18538067,info+element(0x1654ae6b,track)+clusters)
                case['matroska'] = 'aac-ssr-ps-'+case['name']+'-synthetic.mka'
                (DEST/case['matroska']).write_bytes(mka)
                case['edited'] = 'aac-ssr-ps-'+case['name']+'-repeat-synthetic.mp4'
                edited(case['video']['file'],case['edited'],[(1600,-1),(3200,rate//50),(3200,rate//50),(1600,-1)])
                cases.append(case)
    (DEST/'aac-ssr-ps-packets.bin').write_bytes(blob)
    (DEST/'aac-ssr-ps.json').write_text(json.dumps(dict(controls=controls,cases=cases,payloads=video['sbr_payloads'],
        acceptance='Enabled mono native SSR/PS waveform, both EOF frames and transport timing.',
        qualification='Silent mono SSR window schedules with authored nonzero SBR/PS stereo. SSR PS coupling remains unsupported; broader profiles and nonzero SSR spectral tools require additional qualification.',
        provenance='Own silent SSR sine/KBD core across three valid window schedules; own authored SBR/PS matrix payloads and independent Decimal PS/direct QMF scalar reference. No private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
