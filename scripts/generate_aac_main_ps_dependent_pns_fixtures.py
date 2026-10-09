#!/usr/bin/env python3
"""Authored Main PS dependent PNS; scalar prediction/noise/TNS/IMDCT."""
import json,struct
from generate_aac_main_tools_fixtures import channel,ics,Filterbank,predict,Noise
from generate_aac_main_prediction_fixtures import initial,step,f32
from generate_aac_main_ps_cce_fixtures import tns_filter
from generate_aac_pce_profile_fixtures import program
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture


def main():
    payloads=json.loads((DEST/'aac-ssr-ps.json').read_text())['payloads']
    blob=bytearray();cases=[]
    for source_tns in (False,True):
        for target_tns in (False,True):
            for point in (0,1):
                noise=Noise();histories={t:[initial() for _ in range(8)] for t in (1,15)};bad_histories={t:[initial() for _ in range(8)] for t in (1,15)}
                bank=Filterbank();bad_bank=Filterbank();pcm=[];bad_pcm=[];rows=[]
                for i in range(12):
                    shape=i%2;active=i>=2;reset=1 if i==10 else None;elements=[]
                    mixed=[0.]*8;wrong=[0.]*8
                    for tag in ([1,15] if i%2==0 else [15,1]):
                        sign=-1 if (i+(tag==15))%2 else 1
                        values=[[sign,-sign,sign,-sign]+[0,sign,-sign,sign]]
                        if tag==15:values=[[0,sign,-sign,sign]+[sign,0,-sign,0]]
                        books=[1,13 if 3<=i<=8 else 1];energy=50 if tag==1 else 54
                        raw=channel(0,books,values,energy=energy,info=ics(0,2,active,reset,shape),tns=(tag==15,1) if source_tns else None)
                        elements.append('010'+field(tag,4)+'0'+'000'+'0'+'0000'+field(point==1,1)+'000'+raw)
                        spectrum=[float(v*1024) for v in values[0]]
                        if books[1]==13:spectrum[4:]=noise.band(energy)
                        good=predict(histories[tag],spectrum,active,books,reset)
                        if books[1]==13:
                            first=bad_histories[tag][:4];bad=predict(first,spectrum[:4],active,[1],reset)+spectrum[4:];bad_histories[tag][:4]=first
                            for k in range(4,8):_,bad_histories[tag][k]=step(bad_histories[tag][k],0.,False)
                        else:bad=predict(bad_histories[tag],spectrum,active,books,reset)
                        if source_tns:good=tns_filter(good,tag==15);bad=tns_filter(bad,tag==15)
                        mixed=[f32(a+b) for a,b in zip(mixed,good)];wrong=[f32(a+b) for a,b in zip(wrong,bad)]
                    if target_tns and point==0:mixed=tns_filter(mixed);wrong=tns_filter(wrong)
                    pcm.extend(bank.run(0,[mixed],shape));bad_pcm.extend(bad_bank.run(0,[wrong],shape))
                    payload=payloads[i%3];target='0000000'+channel(0,[0,0],[[0]*8],info=ics(0,2,False,shape=shape),tns=(False,1) if target_tns else None)+fill(bytes.fromhex(payload))
                    raw=packed((''.join(elements)+target if i%2 else target+''.join(elements))+'111')
                    rows.append(dict(offset=len(blob),bytes=len(raw),payload=payload));blob.extend(raw)
                name=f'{int(source_tns)}-{int(target_tns)}-{point}'
                core=f'aac-main-ps-dependent-pns-{name}-core.f32le';bad=f'aac-main-ps-dependent-pns-{name}-retained-predictor.f32le'
                for file,data in ((core,pcm),(bad,bad_pcm)):(DEST/file).write_bytes(struct.pack('<'+str(len(data))+'f',*data))
                for rate in (24000,48000):
                    prefix=field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(1,5)+'000'
                    c=dict(name=name+'-'+str(rate),source_tns=source_tns,tns=target_tns,point=point,asc=packed(program(prefix,1,1,coupling=[(False,1),(False,15)])).hex(),frames=rows,
                           reference=core,discarded=bad,slots=16,bands=32*(rate//24000),channels=2,pcm_offset=0,samples=12288*(rate//24000),container_rate=rate,container_frame_samples=1024*(rate//24000))
                    c['video']=video_fixture([c],blob,channels=2,filename=f'aac-main-ps-dependent-pns-{name}-{rate}-synthetic.mp4');cases.append(c)
    (DEST/'aac-main-ps-dependent-pns-packets.bin').write_bytes(blob)
    (DEST/'aac-main-ps-dependent-pns.json').write_text(json.dumps(dict(cases=cases,provenance='Own two-source Main PS dependent coupling points 0/1; scalar global wire-order PNS and Main predictor/reset, directional source and target TNS, f32 spectral addition, direct IMDCT; authored target PS; no private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
