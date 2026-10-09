#!/usr/bin/env python3
"""Authored Main PS dependent PNS; scalar prediction/noise/TNS/IMDCT."""
import json,struct
from generate_aac_main_tools_fixtures import channel,ics,Filterbank,predict,Noise,sc,tuple_bits
from generate_aac_main_prediction_fixtures import initial,step,f32
from generate_aac_main_ps_cce_fixtures import tns_filter
from generate_aac_pce_profile_fixtures import program
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture


def group_channel(books,values,energy,shape,groups):
    assert sum(groups)==8
    grouping=''.join('1'*(length-1)+('0' if i+1<len(groups) else '') for i,length in enumerate(groups))
    layouts=[books if books==[0,0] else ([1,13] if i%2==0 else [13,1]) for i in range(len(groups))]
    sections=''.join(field(book,4)+field(1,3) for row in layouts for book in row)
    scales='';first=True
    for row in layouts:
        for book in row:
            if book==1:scales+=sc(0)
            elif book==13:
                scales+=field(energy-50+256,9) if first else sc(0);first=False
    raw=field(140,8)+'0'+field(2,2)+field(shape,1)+field(2,4)+grouping+sections+scales+'000'
    first_window=0
    for length,row in zip(groups,layouts):
        for band,book in enumerate(row):
            if book==1:
                for w in range(first_window,first_window+length):raw+=tuple_bits(values[w][4*band:4*band+4])
        first_window+=length
    return raw,layouts


def main():
    payloads=json.loads((DEST/'aac-ssr-ps.json').read_text())['payloads']
    blob=bytearray();cases=[]
    for source_tns in (False,True):
        for target_tns in (False,True):
            for point in (0,1):
                noise=Noise();histories={t:[initial() for _ in range(8)] for t in (1,15)};bad_histories={t:[initial() for _ in range(8)] for t in (1,15)}
                bank=Filterbank();bad_bank=Filterbank();pcm=[];bad_pcm=[];rows=[]
                for i in range(12):
                    seq=[0,0,0,1,2,2,3,0,0,0,0,0][i];count=8 if seq==2 else 1
                    shape=i%2;active=i>=2 and seq!=2;reset=1 if i==10 else None;elements=[]
                    mixed=[[0.]*8 for _ in range(count)];wrong=[[0.]*8 for _ in range(count)]
                    for tag in ([1,15] if i%2==0 else [15,1]):
                        sign=-1 if (i+(tag==15))%2 else 1
                        values=[[sign,-sign,sign,-sign]+[0,sign,-sign,sign]]
                        if tag==15:values=[[0,sign,-sign,sign]+[sign,0,-sign,0]]
                        values=[[v*(-1)**w for v in values[0]] for w in range(count)]
                        books=[1,13 if 3<=i<=8 else 1];energy=50 if tag==1 else 54
                        groups=([1,3,4] if tag==1 else [2,1,2,3]) if i==4 else ([1]*8 if tag==1 else [4,4])
                        raw=channel(seq,books,values,energy=energy,info=ics(seq,2,active,reset,shape),tns=(tag==15,1) if source_tns and seq!=2 else None)
                        if seq==2:raw,layouts=group_channel(books,values,energy,shape,groups)
                        elements.append('010'+field(tag,4)+'0'+'000'+'0'+'0000'+field(point==1,1)+'000'+raw)
                        spectra=[[float(v*1024) for v in row] for row in values]
                        if seq==2:
                            start=0
                            for length,layout in zip(groups,layouts):
                                for band,book in enumerate(layout):
                                    if book==13:
                                        for w in range(start,start+length):spectra[w][4*band:4*band+4]=noise.band(energy)
                                start+=length
                        elif books[1]==13:spectra[0][4:]=noise.band(energy)
                        if seq==2:
                            histories[tag]=[initial() for _ in range(8)]
                            good=[row[:] for row in spectra];bad=[row[:] for row in spectra]
                        else:
                            good=[predict(histories[tag],spectra[0],active,books,reset)]
                            bad=[predict(bad_histories[tag],spectra[0],active,books,reset)]
                            if source_tns:good=[tns_filter(good[0],tag==15)];bad=[tns_filter(bad[0],tag==15)]
                        for w in range(count):
                            mixed[w]=[f32(a+b) for a,b in zip(mixed[w],good[w])]
                            wrong[w]=[f32(a+b) for a,b in zip(wrong[w],bad[w])]
                    if target_tns and point==0 and seq!=2:mixed=[tns_filter(mixed[0])];wrong=[tns_filter(wrong[0])]
                    pcm.extend(bank.run(seq,mixed,shape));bad_pcm.extend(bad_bank.run(seq,wrong,shape))
                    payload=payloads[i%3];target='0000000'+channel(seq,[0,0],[[0]*8 for _ in range(count)],info=ics(seq,2,False,shape=shape),tns=(False,1) if target_tns and seq!=2 else None)+fill(bytes.fromhex(payload))
                    if seq==2:target='0000000'+group_channel([0,0],[[0]*8 for _ in range(8)],50,shape,[3,2,3] if i==4 else [8])[0]+fill(bytes.fromhex(payload))
                    raw=packed((''.join(elements)+target if i%2 else target+''.join(elements))+'111')
                    rows.append(dict(offset=len(blob),bytes=len(raw),payload=payload));blob.extend(raw)
                name=f'{int(source_tns)}-{int(target_tns)}-{point}'
                core=f'aac-main-ps-dependent-pns-grouped-{name}-core.f32le';bad=f'aac-main-ps-dependent-pns-grouped-{name}-retained-predictor.f32le'
                for file,data in ((core,pcm),(bad,bad_pcm)):(DEST/file).write_bytes(struct.pack('<'+str(len(data))+'f',*data))
                for rate in (24000,48000):
                    prefix=field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(1,5)+'000'
                    c=dict(name=name+'-'+str(rate),source_tns=source_tns,tns=target_tns,point=point,asc=packed(program(prefix,1,1,coupling=[(False,1),(False,15)])).hex(),frames=rows,
                           reference=core,discarded=bad,slots=16,bands=32*(rate//24000),channels=2,pcm_offset=0,samples=12288*(rate//24000),container_rate=rate,container_frame_samples=1024*(rate//24000))
                    c['video']=video_fixture([c],blob,channels=2,filename=f'aac-main-ps-dependent-pns-grouped-{name}-{rate}-synthetic.mp4');cases.append(c)
    (DEST/'aac-main-ps-dependent-pns-grouped-packets.bin').write_bytes(blob)
    (DEST/'aac-main-ps-dependent-pns-grouped.json').write_text(json.dumps(dict(cases=cases,provenance='Own unequal source/target window groups with alternating PNS band layouts and shared long/start/short/stop transitions with two-source Main PS dependent coupling points 0/1; scalar global wire-order PNS and Main short-window reset control, directional source and target TNS, f32 spectral addition, direct IMDCT; authored target PS; no private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
