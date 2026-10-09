#!/usr/bin/env python3
"""Own LTP/PS tag histories, arrivals/returns and source SBR, offline only."""
import json,struct
from generate_aac_main_tools_fixtures import channel,ics
from generate_aac_ltp_transition_fixtures import Oracle,SEQUENCES
from generate_aac_pce_profile_fixtures import program
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def element(n,i,tag):
    sequences=SEQUENCES if tag!=15 else [0,0,1,2,2,3,0,0,1,2,3,0]
    seq=sequences[i];shape=(i+(tag==15))%2;active=i>=1 and seq!=2
    lag=n-13*(i%3)-(17 if tag==15 else 0);coefficient=(i+(3 if tag==15 else 0))%8;used=[True,i%3!=1]
    q=[[(-1 if (i+w+k+tag)%3==0 else 1 if (i+w+k+tag)%3==1 else 0) for k in range(8)] for w in range(8 if seq==2 else 1)]
    h=ics(seq,2,False,shape=shape)
    if active:h=h[:-1]+'11'+field(lag,11)+field(coefficient,3)+''.join(str(int(v)) for v in used)
    return channel(seq,[1,1],q,info=h),(seq,shape,q,active,lag,coefficient,used)

def main():
    syntax=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes();dsp=json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases']
    blob=bytearray();cases=[]
    schedules=[('full',False)]+[(s,d) for s in ('return','arrival','both-return','both-arrival') for d in (False,True)]
    for n in (960,1024):
        slots=n//64
        payloads=json.loads((DEST/'aac-ssr-ps.json').read_text())['payloads'] if n==1024 else json.loads((DEST/'aac-sbr-ps-30-oracles.json').read_text())['videos'][0]['sbr_payloads']
        geometry=next(c for c in dsp if c['slots']==slots and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
        channels={};cores={};discarded={}
        for tag in (0,1,15):
            oracle=Oracle(n);disabled=Oracle(n);pcm=[];wrong=[];channels[tag]=[]
            for i in range(12):
                bits,args=element(n,i,tag);channels[tag].append(bits)
                pcm.extend(oracle.run(*args));args=list(args);args[3]=False;wrong.extend(disabled.run(*args))
            core=f'aac-ltp-ps-independent-{n}-{tag}-core.f32le';bad=f'aac-ltp-ps-independent-{n}-{tag}-disabled.f32le'
            for file,data in ((core,pcm),(bad,wrong)):(DEST/file).write_bytes(struct.pack('<'+str(len(data))+'f',*data))
            cores[tag]=core;discarded[tag]=bad
        for source_sbr in (False,True):
            for schedule,dynamic in schedules:
                rows=[];ordinals={1:0,15:0}
                initial_tags=(() if schedule=='both-arrival' else (15,)) if schedule in ('arrival','both-arrival') and dynamic else (1,15)
                previous=initial_tags
                for i in range(12):
                    present=(() if schedule.startswith('both-') else (15,)) if (schedule.endswith('return') and 4<=i<7) or (schedule.endswith('arrival') and i<3) else (1,15)
                    roster=present if dynamic else (1,15)
                    payload=payloads[i%3];target='0000000'+channels[0][i]+fill(bytes.fromhex(payload))
                    sources=[];metadata=[]
                    for tag in present:
                        ordinal=ordinals[tag];ordinals[tag]+=1;raw=b''
                        if source_sbr and tag==1:
                            f=geometry['frames'][ordinal%3];raw=syntax[f['offset']:f['offset']+f['byte_length']]
                        sources.append('010'+field(tag,4)+'1'+'000'+'0'+'0000'+'0010'+channels[tag][ordinal]+fill(raw))
                        metadata.append(dict(tag=tag,core_index=ordinal,payload=raw.hex(),reference=cores[tag],discarded=discarded[tag]))
                    if i%2:sources.reverse();metadata.reverse()
                    raw=packed((''.join(sources)+target if i%2 else target+''.join(sources))+'111')
                    if roster!=previous:raw=packed(program('101',4,1,coupling=[(True,tag) for tag in roster]))+raw
                    previous=roster
                    rows.append(dict(offset=len(blob),bytes=len(raw),payload=payload,sources=metadata));blob.extend(raw)
                for rate in (24000,48000):
                    prefix=field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(4,5)+field(n==960,1)+'00'
                    name=f'{n}-{int(source_sbr)}-{rate}-{schedule}-{int(dynamic)}'
                    c=dict(name=name,n=n,target_core=cores[0],schedule=schedule,dynamic=dynamic,source_sbr=source_sbr,asc=packed(program(prefix,4,1,coupling=[(True,tag) for tag in initial_tags])).hex(),frames=rows,slots=slots,bands=32*(rate//24000),channels=2,pcm_offset=0,samples=12*n*(rate//24000),container_rate=rate,container_frame_samples=n*(rate//24000))
                    c['video']=video_fixture([c],blob,channels=2,filename=f'aac-ltp-ps-independent-{name}-synthetic.mp4');cases.append(c)
    (DEST/'aac-ltp-ps-independent-packets.bin').write_bytes(blob)
    (DEST/'aac-ltp-ps-independent.json').write_text(json.dumps(dict(cases=cases,provenance='Own active LTP target and independently coded source tags 1/15, scalar float history/window transforms; pause on absence, PCE arrival/return rosters and authored source SBR. No private media, FFmpeg or network.'),indent=2)+'\n')
    print(len(cases),'original LTP/PS independent-source videos')
if __name__=='__main__':main()
