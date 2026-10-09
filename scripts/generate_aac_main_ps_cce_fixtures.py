#!/usr/bin/env python3
"""Own Main PS dependent CCE; independent scalar predictor and IMDCT."""
import json, struct
from generate_aac_main_tools_fixtures import channel, ics, Filterbank, predict, SEQUENCES
from generate_aac_main_prediction_fixtures import initial, f32
from generate_aac_pce_profile_fixtures import program
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture


def main():
    payloads=json.loads((DEST/'aac-ssr-ps.json').read_text())['payloads']
    blob=bytearray(); cases=[]
    for point in (0,1):
        banks={tag:[initial() for _ in range(4)] for tag in (1,15)}
        synthesis=Filterbank(); wrong_synthesis=Filterbank(); pcm=[]; wrong=[]; rows=[]
        for i,seq in enumerate(SEQUENCES):
            shape=i%2; count=8 if seq==2 else 1
            combined=[[0.]*4 for _ in range(count)]; discarded=[[0.]*4 for _ in range(count)]; sources=[]
            for tag in (1,15):
                active=i>=2 and seq!=2; reset=1 if (tag==1 and i==9) or (tag==15 and i==10) else None
                values=[[1,-1,1,-1] if (i+w+(tag==15))%2==0 else [-1,1,-1,1] for w in range(count)]
                # Distinct second-source residuals make accidental bank sharing observable.
                if tag==15: values=[[row[0],0,row[2],0] for row in values]
                source=channel(seq,[1],values,info=ics(seq,1,active,reset,shape))
                sources.append('010'+field(tag,4)+'0'+'000'+'0'+'0000'+field(point==1,1)+'000'+source)
                spectra=[[float(v*1024) for v in row] for row in values]
                bad=[row[:] for row in spectra]
                if seq==2: banks[tag]=[initial() for _ in range(4)]
                else:
                    spectra[0]=predict(banks[tag],spectra[0],active,[1],reset)
                    bad[0]=predict([initial() for _ in range(4)],bad[0],active,[1],reset)
                for w in range(count):
                    for k in range(4):
                        combined[w][k]=f32(combined[w][k]+spectra[w][k])
                        discarded[w][k]=f32(discarded[w][k]+bad[w][k])
            target='0000000'+channel(seq,[0],[[0]*4 for _ in range(count)],info=ics(seq,1,False,shape=shape))
            payload=payloads[i%3]; target+=fill(bytes.fromhex(payload))
            if i%2: sources.reverse()
            raw=packed((''.join(sources)+target if i%2 else target+''.join(sources))+'111')
            rows.append(dict(offset=len(blob),bytes=len(raw),payload=payload)); blob.extend(raw)
            pcm.extend(synthesis.run(seq,combined,shape)); wrong.extend(wrong_synthesis.run(seq,discarded,shape))
        core=f'aac-main-ps-cce-{point}-core.f32le'; bad=f'aac-main-ps-cce-{point}-discarded.f32le'
        for name,data in ((core,pcm),(bad,wrong)):
            (DEST/name).write_bytes(struct.pack('<'+str(len(data))+'f',*data))
        for rate in (24000,48000):
            prefix=field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(1,5)+'000'
            c=dict(name=f'{point}-{rate}',point=point,asc=packed(program(prefix,1,1,coupling=[(False,1),(False,15)])).hex(),frames=rows,
                reference=core,discarded=bad,slots=16,bands=32*(rate//24000),channels=2,pcm_offset=0,samples=12288*(rate//24000),container_rate=rate,container_frame_samples=1024*(rate//24000))
            c['video']=video_fixture([c],blob,channels=2,filename=f'aac-main-ps-cce-{point}-{rate}-synthetic.mp4'); cases.append(c)
    (DEST/'aac-main-ps-cce-packets.bin').write_bytes(blob)
    (DEST/'aac-main-ps-cce.json').write_text(json.dumps(dict(cases=cases,provenance='Own Main PS two-source CCE tags 1/15, independent scalar per-tag prediction, distinct reset groups and residuals, sine/KBD and short-window transitions; no private media, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__': main()
