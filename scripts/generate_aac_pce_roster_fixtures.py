#!/usr/bin/env python3
"""Own dynamic Main/LC PCE CCE programs and scalar prediction/IMDCT PCM."""
import json, struct, math
from generate_aac_pce_profile_fixtures import program, config
from generate_aac_main_tools_fixtures import channel, ics, Filterbank, predict, SEQUENCES
from generate_aac_main_prediction_fixtures import initial, add, f32
from generate_he_aac_packet_fixtures import DEST, field, packed, video_fixture


def main():
    blob=bytearray();gold=bytearray();cases=[]
    for obj in (1,2):
        for schedule in ('return','return-long','arrival'):
            rosters=([(1,15)]*3+[(15,)]*3+[(1,15)]*6 if schedule=='return'
                     else [(1,15)]*5+[(15,)]*3+[(1,15)]*4 if schedule=='return-long'
                     else [()]*2+[(15,)]*2+[(1,15)]*8)
            banks={tag:Filterbank() for tag in (0,1,15)}
            histories={tag:[initial() for _ in range(4)] for tag in (0,1,15)}
            wrong_banks={tag:Filterbank() for tag in (1,15)}
            wrong_histories={tag:[initial() for _ in range(4)] for tag in (1,15)}
            predictor_banks={tag:Filterbank() for tag in (1,15)}
            predictor_histories={tag:[initial() for _ in range(4)] for tag in (1,15)}
            ordinals={1:0,15:0};bodies=[];pcm=[];wrong_pcm=[];predictor_pcm=[]
            for frame,seq in enumerate(SEQUENCES):
                active=obj==1 and frame>=3 and seq!=2
                reset=1 if obj==1 and frame==10 else None
                count=8 if seq==2 else 1
                values=[[0,1,-1,0] for _ in range(count)]
                target='0000000'+channel(seq,[1],values,info=ics(seq,1,active,reset,frame%2))
                reconstructed=[[float(v*1024) for v in row] for row in values]
                if obj==1:
                    if seq==2:histories[0]=[initial() for _ in range(4)]
                    else:reconstructed[0]=predict(histories[0],reconstructed[0],active,[1],reset)
                output=banks[0].run(seq,reconstructed,frame%2)
                wrong_output=output[:];predictor_output=output[:]
                for tag in (1,15):
                    if tag not in rosters[frame]:
                        wrong_banks[tag]=Filterbank();wrong_histories[tag]=[initial() for _ in range(4)]
                        predictor_histories[tag]=[initial() for _ in range(4)]
                sources=[]
                selected=list(rosters[frame])
                if frame%2:selected.reverse()
                for tag in selected:
                    ordinal=ordinals[tag];ordinals[tag]+=1
                    sequences=([0,0,1,2,2,3,0,0,1,2,3,0] if tag==1 else SEQUENCES)
                    if tag==1 and schedule=='return-long':sequences=[0]*12
                    source_seq=sequences[ordinal];shape=(ordinal+int(tag==15))%2
                    active=obj==1 and ordinal>=3 and source_seq!=2
                    reset=1 if obj==1 and ordinal==10 else None
                    values=[]
                    for w in range(8 if source_seq==2 else 1):
                        sign=-1 if (ordinal+w)%2 else 1
                        values.append([sign,-sign,sign,-sign] if tag==1 else [0,sign,-sign,sign])
                    raw=channel(source_seq,[1],values,info=ics(source_seq,1,active,reset,shape))
                    sources.append('010'+field(tag,4)+'1'+'000'+'0'+'0000'+'0'+'0'+'10'+raw)
                    reconstructed=[[float(v*1024) for v in row] for row in values]
                    if obj==1:
                        if source_seq==2:histories[tag]=[initial() for _ in range(4)]
                        else:reconstructed[0]=predict(histories[tag],reconstructed[0],active,[1],reset)
                    source=banks[tag].run(source_seq,reconstructed,shape)
                    output=[add(a,b) for a,b in zip(output,source)]
                    wrong=[[float(v*1024) for v in row] for row in values]
                    if obj==1:
                        if source_seq==2:wrong_histories[tag]=[initial() for _ in range(4)]
                        else:wrong[0]=predict(wrong_histories[tag],wrong[0],active,[1],reset)
                    wrong_source=wrong_banks[tag].run(source_seq,wrong,shape)
                    wrong_output=[add(a,b) for a,b in zip(wrong_output,wrong_source)]
                    pred_only=[[float(v*1024) for v in row] for row in values]
                    if obj==1:
                        if source_seq==2:predictor_histories[tag]=[initial() for _ in range(4)]
                        else:pred_only[0]=predict(predictor_histories[tag],pred_only[0],active,[1],reset)
                    predictor_source=predictor_banks[tag].run(source_seq,pred_only,shape)
                    predictor_output=[add(a,b) for a,b in zip(predictor_output,predictor_source)]
                bodies.append(packed(target+''.join(sources)+'111'));pcm.extend(output);wrong_pcm.extend(wrong_output);predictor_pcm.extend(predictor_output)
            offset=len(gold);gold.extend(struct.pack('<'+str(len(pcm))+'f',*pcm))
            wrong_offset=len(gold);gold.extend(struct.pack('<'+str(len(wrong_pcm))+'f',*wrong_pcm))
            predictor_offset=len(gold);gold.extend(struct.pack('<'+str(len(predictor_pcm))+'f',*predictor_pcm))
            for dynamic in (False,True):
                rows=[];previous=rosters[0] if dynamic else (1,15)
                for i,body in enumerate(bodies):
                    roster=rosters[i] if dynamic else (1,15)
                    raw=body
                    if roster != previous:
                        raw=packed(program('101',obj,1,coupling=[(True,tag) for tag in roster]))+raw
                    rows.append(dict(offset=len(blob),bytes=len(raw),roster=list(roster)));blob.extend(raw);previous=roster
                name=f'{obj}-{schedule}-'+('dynamic' if dynamic else 'static')
                initial_tags=rosters[0] if dynamic else (1,15)
                case=dict(name=name,object_type=obj,dynamic=dynamic,schedule=schedule,point=3,tns=False,
                          asc=config(obj,1,coupling=[(True,tag) for tag in initial_tags]).hex(),frames=rows,
                          slots=16,bands=32,channels=1,container_rate=24000,container_frame_samples=1024,
                          samples=len(pcm),pcm_offset=offset,pcm_bytes=len(pcm)*4,discarded_history_pcm_offset=wrong_offset,discarded_predictor_pcm_offset=predictor_offset)
                case['video']=video_fixture([case],blob,filename='aac-pce-roster-'+name+'-synthetic.mp4');cases.append(case)
    # Dependent sources share target window geometry and mix before its
    # filterbank. Target TNS distinguishes point 0 from point 1 numerically.
    def tns_filter(values):
        previous=0.;result=[]
        for value in values:
            value=value-math.sin(math.pi/7)*previous
            result.append(f32(value));previous=value
        return result
    for obj in (1,2):
        for point in (0,1):
            for schedule in ('return','arrival'):
                for tns in (False,True):
                    rosters=([(1,15)]*5+[(15,)]*3+[(1,15)]*4 if schedule=='return'
                             else [()]*2+[(15,)]*2+[(1,15)]*8)
                    histories={tag:[initial() for _ in range(4)] for tag in (0,1,15)}
                    bank=Filterbank();ordinals={1:0,15:0};bodies=[];pcm=[]
                    for frame,seq in enumerate([0]*12 if tns else SEQUENCES):
                        shape=frame%2;active=obj==1 and frame>=3 and seq!=2
                        reset=1 if obj==1 and frame==10 else None
                        count=8 if seq==2 else 1;values=[[0,1,-1,0] for _ in range(count)]
                        target='0000000'+channel(seq,[1],values,info=ics(seq,1,active,reset,shape),tns=(False,1) if tns else None)
                        spectrum=[[float(v*1024) for v in row] for row in values]
                        if obj==1:
                            if seq==2:histories[0]=[initial() for _ in range(4)]
                            else:spectrum[0]=predict(histories[0],spectrum[0],active,[1],reset)
                        if tns and point==1:spectrum=[tns_filter(spectrum[0])]
                        selected=list(rosters[frame])
                        if frame%2:selected.reverse()
                        sources=[]
                        for tag in selected:
                            ordinal=ordinals[tag];ordinals[tag]+=1
                            active=obj==1 and ordinal>=3 and seq!=2
                            reset=1 if obj==1 and ordinal==10 else None
                            values=[]
                            for w in range(count):
                                sign=-1 if (ordinal+w)%2 else 1
                                values.append([sign,-sign,sign,-sign] if tag==1 else [0,sign,-sign,sign])
                            raw=channel(seq,[1],values,info=ics(seq,1,active,reset,shape))
                            sources.append('010'+field(tag,4)+'0'+'000'+'0'+'0000'+field(point==1,1)+'0'+'10'+raw)
                            reconstructed=[[float(v*1024) for v in row] for row in values]
                            if obj==1:
                                if seq==2:histories[tag]=[initial() for _ in range(4)]
                                else:reconstructed[0]=predict(histories[tag],reconstructed[0],active,[1],reset)
                            spectrum=[[add(a,b) for a,b in zip(x,y)] for x,y in zip(spectrum,reconstructed)]
                        if tns and point==0:spectrum=[tns_filter(spectrum[0])]
                        pcm.extend(bank.run(seq,spectrum,shape))
                        bodies.append(packed(target+''.join(sources)+'111'))
                    offset=len(gold);gold.extend(struct.pack('<'+str(len(pcm))+'f',*pcm))
                    for dynamic in (False,True):
                        rows=[];previous=rosters[0] if dynamic else (1,15)
                        for i,body in enumerate(bodies):
                            roster=rosters[i] if dynamic else (1,15);raw=body
                            if roster!=previous:raw=packed(program('101',obj,1,coupling=[(False,tag) for tag in roster]))+raw
                            rows.append(dict(offset=len(blob),bytes=len(raw),roster=list(roster)));blob.extend(raw);previous=roster
                        name=f'{obj}-dependent-{point}-{schedule}-{int(tns)}-'+('dynamic' if dynamic else 'static')
                        initial_tags=rosters[0] if dynamic else (1,15)
                        case=dict(name=name,object_type=obj,point=point,tns=tns,dynamic=dynamic,schedule=schedule,
                                  asc=config(obj,1,coupling=[(False,tag) for tag in initial_tags]).hex(),frames=rows,
                                  slots=16,bands=32,channels=1,container_rate=24000,container_frame_samples=1024,
                                  samples=len(pcm),pcm_offset=offset,pcm_bytes=len(pcm)*4)
                        case['video']=video_fixture([case],blob,filename='aac-pce-roster-'+name+'-synthetic.mp4');cases.append(case)
    (DEST/'aac-pce-roster-packets.bin').write_bytes(blob)
    (DEST/'aac-pce-roster-pcm.f32le').write_bytes(gold)
    (DEST/'aac-pce-roster.json').write_text(json.dumps(dict(cases=cases,
        provenance='Own Main/LC residual spectra, independent scalar predictor and direct IMDCT windows/overlap. Tag-keyed source ordinals pause on absence. No private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
