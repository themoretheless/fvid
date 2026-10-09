#!/usr/bin/env python3
"""Authored nonzero Main core + PS; independent scalar Main prediction PCM."""
import json,struct
from generate_aac_main_tools_fixtures import channel,ics,Filterbank,predict,SEQUENCES
from generate_aac_main_prediction_fixtures import initial
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture


def main():
    payloads=json.loads((DEST/'aac-ssr-ps.json').read_text())['payloads']
    bank=Filterbank();history=[initial() for _ in range(4)];blob=bytearray();core_rows=[];ps_rows=[];pcm=[]
    for i,seq in enumerate(SEQUENCES):
        active=i>=3 and seq!=2;reset=1 if i==10 else None
        values=[([1,-1,1,-1] if (i+w)%2==0 else [-1,1,-1,1]) for w in range(8 if seq==2 else 1)]
        target='0000000'+channel(seq,[1],values,info=ics(seq,1,active,reset,i%2))
        raw=packed(target+'111');core_rows.append(dict(offset=len(blob),bytes=len(raw)));blob.extend(raw)
        payload=payloads[i%3];raw=packed(target+fill(bytes.fromhex(payload))+'111')
        ps_rows.append(dict(offset=len(blob),bytes=len(raw),payload=payload));blob.extend(raw)
        spectra=[[float(v*1024) for v in row] for row in values]
        if seq==2:history=[initial() for _ in range(4)]
        else:spectra[0]=predict(history,spectra[0],active,[1],reset)
        pcm.extend(bank.run(seq,spectra,i%2))
    control=dict(name='core',asc=packed(field(1,5)+frequency(24000)+'0001'+'000').hex(),frames=core_rows,
                 slots=16,bands=32,channels=1,pcm_offset=0,samples=12288,container_rate=24000,container_frame_samples=1024)
    control['video']=video_fixture([control],blob,filename='aac-main-ps-core-control-synthetic.mp4')
    cases=[]
    for rate in (24000,48000):
        c=dict(name=str(rate),asc=packed(field(29,5)+frequency(24000)+'0001'+frequency(rate)+field(1,5)+'000').hex(),frames=ps_rows,
               slots=16,bands=32*(rate//24000),channels=2,pcm_offset=0,samples=12288*(rate//24000),container_rate=rate,container_frame_samples=1024*(rate//24000))
        c['video']=video_fixture([c],blob,channels=2,filename='aac-main-ps-'+str(rate)+'-synthetic.mp4');cases.append(c)
    (DEST/'aac-main-ps-core.f32le').write_bytes(struct.pack('<'+str(len(pcm))+'f',*pcm))
    (DEST/'aac-main-ps-packets.bin').write_bytes(blob)
    (DEST/'aac-main-ps.json').write_text(json.dumps(dict(control=control,cases=cases,
        provenance='Own Main residuals, prediction activation/reset, sine/KBD and window transitions; independent scalar prediction/IMDCT. Original authored PS payloads, no private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
