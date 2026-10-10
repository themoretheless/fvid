#!/usr/bin/env python3
"""Own LTP coupling long/start/short/stop, PNS/TNS and scalar histories."""
import json, struct
from generate_aac_ltp_transition_fixtures import Oracle, GAINS
from generate_aac_main_tools_fixtures import channel,ics,Noise,f32,sc,tuple_bits
from generate_aac_main_ps_dependent_pns_tns_orders_fixtures import short_bits,short_filter
from generate_aac_ltp_pns_fixtures import tns_synthesis
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
import math
SEQUENCES=[0,0,1,2,2,2,2,3,0,1,2,2,3,0,0,0]


def short_channel(books,values,shape,groups,reverse,frame):
    grouping=''.join('1'*(length-1)+('0' if i+1<len(groups) else '') for i,length in enumerate(groups))
    sections=''.join(field(book,4)+field(1,3) for _ in groups for book in books)
    scales='';first=True
    for _ in groups:
        for book in books:
            if book==1:scales+=sc(0)
            elif book==13:scales+=field(256,9) if first else sc(0);first=False
    raw=field(140,8)+'0'+'10'+field(shape,1)+field(2,4)+grouping+sections+scales+'0'+'1'+''.join(short_bits(w,frame,reverse) for w in range(8))+'0'
    start=0
    for length in groups:
        for band,book in enumerate(books):
            if book==1:
                for w in range(start,start+length):raw+=tuple_bits(values[w][4*band:4*band+4])
        start+=length
    return raw


def tns(values,seq,reverse,frame):
    if seq==2:return [short_filter(row,w,frame,reverse) for w,row in enumerate(values)]
    return [tns_synthesis(values[0],reverse)]


def predict(oracle,values,seq,shape,active,lag,coef,used,reverse):
    if not active:return [row[:] for row in values]
    n=oracle.n
    estimate=[v*GAINS[coef]*oracle.long_weight(i,seq,shape) for i,v in enumerate(oracle.history[2*n-lag:4*n-lag])]
    prediction=[sum(v*c for v,c in zip(estimate,oracle.cos[n][k])) for k in range(8)]
    history=0.;lpc=math.sin(math.pi/7)
    for k in (range(7,-1,-1) if reverse else range(8)):
        original=prediction[k];prediction[k]=original+lpc*history;history=original
    return [[f32(v+prediction[k]) if used[k//4] else v for k,v in enumerate(values[0])]]


def render(oracle,seq,shape,spectra,stale=False):
    return oracle.run(seq,shape,[[v/1024 for v in row] for row in spectra],False,0,0,[False,False],skip_short_history=stale)


def main():
    blob=bytearray();gold=bytearray();wrong=bytearray();cases=[]
    for n in (960,1024):
        for point in (0,1):
            source=Oracle(n);bad=Oracle(n);target=Oracle(n);bad_target=Oracle(n);noise=Noise();rows=[];controls=[];start=len(gold);short_index=0
            for frame,seq in enumerate(SEQUENCES):
                shape=frame%2;active=seq!=2 and frame>=1;lag=n-13*(frame%3);coef=frame%8;reverse=bool(frame%2)
                count=8 if seq==2 else 1;books=[1,13 if 3<=frame<=12 else 1];used=[True,books[1]!=13]
                q=[[(frame+w+k)%3-1 for k in range(8)] for w in range(count)];tq=[[(frame+w+k+1)%3-1 for k in range(8)] for w in range(count)]
                index=short_index
                if seq==2:short_index+=1
                def encode(flags):
                    if seq==2:
                        body=short_channel(books,q,shape,[1,3,4],reverse,index)
                        dest=short_channel([1,1],tq,shape,[3,2,3],not reverse,index)
                    else:
                        info=ics(seq,2,False,shape=shape)
                        if active:info=info[:-1]+'11'+field(lag,11)+field(coef,3)+''.join(field(v,1) for v in flags)
                        body=channel(seq,books,q,info=info,tns=(reverse,1))
                        dest=channel(seq,[1,1],tq,info=ics(seq,2,False,shape=shape),tns=(not reverse,1))
                    cce='0100001'+'0'+'000'+'0'+'0000'+field(point==1,1)+'0'+'10'+body
                    dest='0000000'+dest
                    return packed((cce+dest if frame%2 else dest+cce)+'111')
                for flags,listing in (([True,True],rows),(used,controls)):
                    raw=encode(flags);listing.append(dict(offset=len(blob),bytes=len(raw),sequence=seq,short_index=index));blob.extend(raw)
                values=[[float(v*1024) for v in row] for row in q]
                if books[1]==13:
                    for row in values:row[4:8]=noise.band(50)
                good=tns(predict(source,values,seq,shape,active,lag,coef,used,reverse),seq,reverse,index)
                stale=tns(predict(bad,values,seq,shape,active,lag,coef,used,reverse),seq,reverse,index)
                render(source,seq,shape,good);render(bad,seq,shape,stale,True)
                def output(bank,src):
                    residual=[[float(v*1024) for v in row] for row in tq]
                    if point==1:residual=tns(residual,seq,not reverse,index)
                    mixed=[[f32(a+b) for a,b in zip(dst,s)] for dst,s in zip(residual,src)]
                    if point==0:mixed=tns(mixed,seq,not reverse,index)
                    return struct.pack('<'+'f'*n,*render(bank,seq,shape,mixed))
                gold.extend(output(target,good));wrong.extend(output(bad_target,stale))
            prefix=field(4,5)+frequency(24000)+'0000'+field(n==960,1)+'00'
            pce=field(0,4)+field(3,2)+frequency(24000)+field(1,4)+field(0,4)+field(0,4)+field(0,2)+field(0,3)+field(1,4)+'000'+'0'+field(0,4)+'0'+field(1,4)
            asc=packed(prefix+pce+'0'*(-len(prefix+pce)%8)+field(0,8)).hex()
            name=f'{n}-{point}';c=dict(name=name,n=n,point=point,asc=asc,channels=1,frames=rows,control_frames=controls,reference_offset=start,reference_bytes=len(gold)-start,container_rate=24000,container_frame_samples=n,samples=len(SEQUENCES)*n,slots=n//64,bands=32,pcm_offset=0)
            c['video']=video_fixture([c],blob,filename=f'aac-ltp-short-coupling-{name}-synthetic.mp4')
            c['control_video']=video_fixture([dict(c,frames=controls)],blob,filename=f'aac-ltp-short-coupling-{name}-control-synthetic.mp4');cases.append(c)
    for name,data in [('packets.bin',blob),('reference.f32le',gold),('stale-short.f32le',wrong)]: (DEST/f'aac-ltp-short-coupling-{name}').write_bytes(data)
    (DEST/'aac-ltp-short-coupling.json').write_text(json.dumps(dict(cases=cases,provenance='Own 960/1024 long/start/short/stop LTP coupling points0/1, global PNS, unequal source/target window groups and short TNS orders1..7 with3/4-bit compressed/uncompressed coefficients; scalar source and target PCM histories, stale-short-history mutant; no private media, FFmpeg, network or foreign decoder.'),indent=2)+'\n')
    print('generated four LTP short coupling videos and controls')

if __name__=='__main__':main()
