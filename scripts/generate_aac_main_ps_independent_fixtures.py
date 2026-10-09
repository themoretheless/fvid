#!/usr/bin/env python3
"""Own Main PS point-3 sources; independent scalar per-tag core PCM."""
import json,struct
from generate_aac_main_tools_fixtures import channel,ics,Filterbank,predict,SEQUENCES
from generate_aac_main_prediction_fixtures import initial
from generate_aac_pce_profile_fixtures import program
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture


def main():
    payloads=json.loads((DEST/'aac-ssr-ps.json').read_text())['payloads']
    syntax=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    geometry=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases']
                  if c['slots']==16 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    cores={};discarded={};source_channels={}
    for tag in (1,15):
        bank=Filterbank();bad_bank=Filterbank();history=[initial() for _ in range(4)]
        pcm=[];bad_pcm=[];channels=[]
        sequences=SEQUENCES if tag==1 else [0,0,1,2,2,3,0,0,1,2,3,0]
        for i,seq in enumerate(sequences):
            count=8 if seq==2 else 1;shape=(i+(tag==15))%2;active=i>=3 and seq!=2;reset=1 if i==10 and active else None
            values=[[1,-1,1,-1] if (i+w)%2==0 else [-1,1,-1,1] for w in range(count)]
            if tag==15:values=[[0,row[0],row[1],row[2]] for row in values]
            channels.append(channel(seq,[1],values,info=ics(seq,1,active,reset,shape)))
            spectra=[[float(v*1024) for v in row] for row in values];bad=[r[:] for r in spectra]
            if seq==2:history=[initial() for _ in range(4)]
            else:
                spectra[0]=predict(history,spectra[0],active,[1],reset)
                bad[0]=predict([initial() for _ in range(4)],bad[0],active,[1],reset)
            pcm.extend(bank.run(seq,spectra,shape));bad_pcm.extend(bad_bank.run(seq,bad,shape))
        core=f'aac-main-ps-independent-{tag}-core.f32le';wrong=f'aac-main-ps-independent-{tag}-discarded.f32le'
        for name,data in ((core,pcm),(wrong,bad_pcm)):(DEST/name).write_bytes(struct.pack('<'+str(len(data))+'f',*data))
        cores[tag]=core;discarded[tag]=wrong;source_channels[tag]=channels
    blob=bytearray();cases=[]
    schedules=[('full',False),('return',False),('return',True),('arrival',False),('arrival',True),('both-return',False),('both-return',True),('both-arrival',False),('both-arrival',True)]
    for source_sbr in (False,True):
        for schedule,dynamic in schedules:
            rows=[];ordinals={1:0,15:0}
            initial_tags=(() if schedule=='both-arrival' else (15,)) if schedule in ('arrival','both-arrival') and dynamic else (1,15)
            previous=initial_tags
            for i in range(12):
                present=(() if schedule.startswith('both-') else (15,)) if (schedule.endswith('return') and 4<=i<7) or (schedule.endswith('arrival') and i<3) else (1,15)
                roster=present if dynamic else (1,15)
                payload=payloads[i%3];target='0000000'+channel(0,[0],[[0]*4],info=ics(0,1,False))+fill(bytes.fromhex(payload))
                sources=[];metadata=[]
                for tag in present:
                    ordinal=ordinals[tag];ordinals[tag]+=1;raw=b''
                    if source_sbr and tag==1:
                        f=geometry['frames'][ordinal%3];raw=syntax[f['offset']:f['offset']+f['byte_length']]
                    sources.append('010'+field(tag,4)+'1'+'000'+'0'+'0000'+'0010'+source_channels[tag][ordinal]+fill(raw))
                    metadata.append(dict(tag=tag,core_index=ordinal,payload=raw.hex(),reference=cores[tag],discarded=discarded[tag]))
                if i%2:sources.reverse();metadata.reverse()
                raw=packed((''.join(sources)+target if i%2 else target+''.join(sources))+'111')
                if roster!=previous:raw=packed(program('101',1,1,coupling=[(True,tag) for tag in roster]))+raw
                previous=roster
                rows.append(dict(offset=len(blob),bytes=len(raw),payload=payload,sources=metadata));blob.extend(raw)
            for rate in (24000,48000):
                prefix=field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(1,5)+'000'
                suffix='' if schedule=='full' else '-'+schedule+('-dynamic' if dynamic else '-static')
                name=f'{int(source_sbr)}-{rate}{suffix}'
                c=dict(name=name,schedule=schedule,dynamic=dynamic,source_sbr=source_sbr,asc=packed(program(prefix,1,1,coupling=[(True,tag) for tag in initial_tags])).hex(),frames=rows,
                       slots=16,bands=32*(rate//24000),channels=2,pcm_offset=0,samples=12288*(rate//24000),container_rate=rate,container_frame_samples=1024*(rate//24000))
                c['video']=video_fixture([c],blob,channels=2,filename=f'aac-main-ps-independent-{name}-synthetic.mp4');cases.append(c)
    (DEST/'aac-main-ps-independent-packets.bin').write_bytes(blob)
    (DEST/'aac-main-ps-independent.json').write_text(json.dumps(dict(cases=cases,provenance='Own Main PS independent CCE tags 1/15 with distinct windows/shapes/predictor histories; independent scalar core prediction/IMDCT; coded source clocks pause on absence and PCE coupling rosters change; authored source SBR and target PS payloads; no private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
