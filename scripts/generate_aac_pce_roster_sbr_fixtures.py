#!/usr/bin/env python3
"""Own Main/LC independent CCE roster + SBR scalar-clock qualification."""
import json,struct
from generate_aac_pce_profile_fixtures import program
from generate_aac_main_tools_fixtures import channel,ics,Filterbank,predict
from generate_aac_main_prediction_fixtures import initial
from generate_aac_main_sbr_oracle import reference
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture


def main():
    syntax=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    geometry=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases']
                  if c['slots']==16 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    blob=bytearray();cases=[]
    for obj in (1,2):
        for name,present in [('return',[1,1,0,1,1,0]),('arrival',[0,0,1,1,1,1]),('retire',[1,1,0,0,0,0])]:
            bank=Filterbank();history=[initial() for _ in range(4)];ordinal=0;source_bits=[];compact=[]
            for on in present:
                if not on:source_bits.append('');continue
                sign=-1 if ordinal%2 else 1;values=[[sign,-sign,sign,-sign]]
                active=obj==1 and ordinal>=1;reset=1 if obj==1 and ordinal==3 else None
                info=ics(0,1,active,reset,ordinal%2)
                source_bits.append(channel(0,[1],values,info=info))
                spectrum=[float(v*1024) for v in values[0]]
                if obj==1:spectrum=predict(history,spectrum,active,[1],reset)
                compact.extend(bank.run(0,[spectrum],ordinal%2));ordinal+=1
            padded=compact+[0.]*(6144-len(compact))
            for sbr,rate in ((False,24000),(True,24000),(True,48000)):
                ratio=rate//24000
                source_pcm=reference(pcm_override=padded,bands=32*ratio) if sbr else padded
                gold=[];ordinal=0
                for on in present:
                    if on:
                        gold.extend(source_pcm[ordinal*1024*ratio:(ordinal+1)*1024*ratio]);ordinal+=1
                    else:gold.extend([0.]*(1024*ratio))
                filename=f'aac-pce-roster-sbr-{obj}-{name}-{int(sbr)}-{rate}.f64le'
                (DEST/filename).write_bytes(struct.pack('<'+str(len(gold))+'d',*gold))
                discarded=None
                if sbr and name=='return':
                    # Preserve coded core prediction/overlap but reset extension
                    # DSP at return, an intentionally incorrect numerical control.
                    tail=compact[2048:];reset=reference(pcm_override=tail+[0.]*(6144-len(tail)),bands=32*ratio)
                    wrong=gold[:]
                    wrong[3*1024*ratio:5*1024*ratio]=reset[:2*1024*ratio]
                    discarded=f'aac-pce-roster-sbr-{obj}-{name}-{rate}-discarded-dsp.f64le'
                    (DEST/discarded).write_bytes(struct.pack('<'+str(len(wrong))+'d',*wrong))
                for dynamic in (False,True):
                    rows=[];ordinal=0;previous=(1,) if present[0] or not dynamic else ()
                    for i,on in enumerate(present):
                        roster=(1,) if on or not dynamic else ()
                        target='0000000'+channel(0,[0],[[0]*4],info=ics(0,1,False))
                        coded=''
                        if on:
                            coded='0100001'+'1'+'000'+'0'+'0000'+'0'+'0'+'10'+source_bits[i]
                            if sbr:
                                row=geometry['frames'][ordinal%3];payload=syntax[row['offset']:row['offset']+row['byte_length']]
                                coded+='110'+(field(len(payload),4) if len(payload)<15 else '1111'+field(len(payload)-14,8))+''.join(field(b,8) for b in payload)
                            ordinal+=1
                        raw=packed((coded+target if i%2 else target+coded)+'111')
                        if roster!=previous:raw=packed(program('101',obj,1,coupling=[(True,t) for t in roster]))+raw
                        rows.append(dict(offset=len(blob),bytes=len(raw)));blob.extend(raw);previous=roster
                    prefix=(field(5,5)+frequency(24000)+'0000'+frequency(rate)+field(obj,5)+'000' if sbr else field(obj,5)+frequency(24000)+'0000'+'000')
                    initial_tags=() if dynamic and not present[0] else (1,)
                    case_name=f'{obj}-{name}-{int(sbr)}-{rate}-'+('dynamic' if dynamic else 'static')
                    c=dict(name=case_name,object_type=obj,sbr=sbr,dynamic=dynamic,present=present,asc=packed(program(prefix,obj,1,coupling=[(True,t) for t in initial_tags])).hex(),
                           frames=rows,channels=1,slots=16,bands=32*ratio,container_rate=rate,container_frame_samples=1024*ratio,samples=6144*ratio,pcm_offset=0,reference=filename,discarded_sbr_reference=discarded)
                    c['video']=video_fixture([c],blob,filename='aac-pce-roster-sbr-'+case_name+'-synthetic.mp4');cases.append(c)
    (DEST/'aac-pce-roster-sbr-packets.bin').write_bytes(blob)
    (DEST/'aac-pce-roster-sbr.json').write_text(json.dumps(dict(cases=cases,
        provenance='Own residuals and scalar Main prediction/IMDCT, direct QMF/SBR convolution. Independent coded source clock pauses on omitted CCE; target clock continues with zero coupling. No private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
