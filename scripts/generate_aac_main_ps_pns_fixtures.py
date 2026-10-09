#!/usr/bin/env python3
"""Authored Main PS independent PNS/TNS; scalar wire-order noise/core oracle."""
import json,struct
from generate_aac_main_tools_fixtures import channel,ics,Filterbank,predict,Noise
from generate_aac_main_prediction_fixtures import initial,step
from generate_aac_main_ps_cce_fixtures import tns_filter
from generate_aac_pce_profile_fixtures import program
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture


def main():
    payloads=json.loads((DEST/'aac-ssr-ps.json').read_text())['payloads']
    syntax=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    geometry=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases'] if c['slots']==16 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    blob=bytearray();cases=[]
    for tns in (False,True):
        noise=Noise(); banks={tag:Filterbank() for tag in (1,15)}; bad_banks={tag:Filterbank() for tag in (1,15)}
        histories={tag:[initial() for _ in range(8)] for tag in (1,15)};bad_histories={tag:[initial() for _ in range(8)] for tag in (1,15)}
        pcm={1:[],15:[]};bad_pcm={1:[],15:[]};channels={1:[],15:[]}
        for i in range(12):
            order=[1,15] if i%2==0 else [15,1]
            for tag in order:
                shape=(i+(tag==15))%2;active=i>=2;reset=1 if i==10 else None
                sign=-1 if (i+(tag==15))%2 else 1
                values=[[sign,-sign,sign,-sign]+[0,sign,-sign,sign]]
                books=[1,13 if 3<=i<=8 else 1];energy=50 if tag==1 else 54
                channels[tag].append(channel(0,books,values,energy=energy,info=ics(0,2,active,reset,shape),tns=(tag==15,1) if tns else None))
                spectrum=[float(v*1024) for v in values[0]]
                if books[1]==13:spectrum[4:]=noise.band(energy)
                good=predict(histories[tag],spectrum,active,books,reset)
                if books[1]==13:
                    # Intentionally wrong: advance predictor on zeroed noise
                    # input but omit the required per-line reset afterward.
                    first=bad_histories[tag][:4]
                    bad=predict(first,spectrum[:4],active,[1],reset)+spectrum[4:]
                    bad_histories[tag][:4]=first
                    for k in range(4,8):
                        _,bad_histories[tag][k]=step(bad_histories[tag][k],0.,False)
                else:bad=predict(bad_histories[tag],spectrum,active,books,reset)
                if tns:good=tns_filter(good,tag==15);bad=tns_filter(bad,tag==15)
                pcm[tag].extend(banks[tag].run(0,[good],shape));bad_pcm[tag].extend(bad_banks[tag].run(0,[bad],shape))
        names={};wrong={}
        for tag in (1,15):
            names[tag]=f'aac-main-ps-pns-{int(tns)}-{tag}-core.f32le';wrong[tag]=f'aac-main-ps-pns-{int(tns)}-{tag}-retained-predictor.f32le'
            for name,data in ((names[tag],pcm[tag]),(wrong[tag],bad_pcm[tag])):(DEST/name).write_bytes(struct.pack('<'+str(len(data))+'f',*data))
        for source_sbr in (False,True):
            rows=[]
            for i in range(12):
                payload=payloads[i%3];target='0000000'+channel(0,[0],[[0]*4],info=ics(0,1,False))+fill(bytes.fromhex(payload))
                elements=[];metadata=[]
                for tag in ([1,15] if i%2==0 else [15,1]):
                    raw=b''
                    if source_sbr and tag==1:
                        f=geometry['frames'][i%3];raw=syntax[f['offset']:f['offset']+f['byte_length']]
                    elements.append('010'+field(tag,4)+'1'+'000'+'0'+'0000'+'0010'+channels[tag][i]+fill(raw))
                    metadata.append(dict(tag=tag,core_index=i,payload=raw.hex(),reference=names[tag],discarded=wrong[tag]))
                raw=packed((''.join(elements)+target if i%2 else target+''.join(elements))+'111')
                rows.append(dict(offset=len(blob),bytes=len(raw),payload=payload,sources=metadata));blob.extend(raw)
            for rate in (24000,48000):
                prefix=field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(1,5)+'000'
                name=f'{int(tns)}-{int(source_sbr)}-{rate}'
                c=dict(name=name,schedule='full',dynamic=False,source_tns=tns,source_sbr=source_sbr,asc=packed(program(prefix,1,1,coupling=[(True,1),(True,15)])).hex(),frames=rows,
                       slots=16,bands=32*(rate//24000),channels=2,pcm_offset=0,samples=12288*(rate//24000),container_rate=rate,container_frame_samples=1024*(rate//24000))
                c['video']=video_fixture([c],blob,channels=2,filename=f'aac-main-ps-pns-{name}-synthetic.mp4');cases.append(c)
    (DEST/'aac-main-ps-pns-packets.bin').write_bytes(blob)
    (DEST/'aac-main-ps-pns.json').write_text(json.dumps(dict(cases=cases,provenance='Own two-source Main PS PNS band switching, scalar global wire-order noise and per-tag predictor/IMDCT; forward/reverse source TNS optional, authored source SBR/target PS; no private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
