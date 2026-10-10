#!/usr/bin/env python3
"""Own nonzero LFE, direct core/QMF synthesis and ER SBR channel clocks."""
import json,struct,math
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
from generate_aac_main_tools_fixtures import channel,ics
from generate_aac_ltp_transition_fixtures import Oracle
from generate_aac_main_sbr_oracle import reference
from generate_aac_er_multi_sbr_fixtures import LAYOUTS,sbr
from generate_aac_sbr_frequency_oracles import tables

def main(frame_samples=1024):
    prefix="aac-er-lfe-sbr" if frame_samples==1024 else f"aac-er-lfe-sbr-{frame_samples}"
    ga=field(frame_samples==960,1)+"0000"
    _,high,_,_=tables(10,27,0,False,0,0);blob=bytearray();cases=[];oracle=Oracle(frame_samples);cores=[];lfe_core=[];gold=bytearray()
    for frame in range(6):
        q=([1,-1,1,-1] if frame%3==0 else [-1,0,1,0] if frame%3==1 else [0,0,0,0])
        cores.append(channel(0,[1],[q],info=ics(0,1,False)))
        lfe_core.extend(oracle.run(0,0,[q+[0]*4],False,frame_samples,0,[False,False]))
    references={}
    for bands in (32,64):
        noise=reference(pcm_override=[0.]*(6*frame_samples),bands=bands,frame_samples=frame_samples)
        lfe=reference(pcm_override=lfe_core,bands=bands,sbr_frames=[False]*6,frame_samples=frame_samples)
        wrong=reference(pcm_override=lfe_core,bands=bands,sbr_frames=[False]*6,qmf_delay=0,frame_samples=frame_samples)
        assert max(abs(a-b) for a,b in zip(lfe,wrong))>1e-5
        references[bands]=(noise,lfe,wrong)
    silent=field(100,8)+'0000'+'000000'+'0'+'000'
    def store(data):
        row=dict(offset=len(blob),bytes=len(data));blob.extend(data);return row
    for aot in (17,19):
     for bands,rate in ((32,24000),(64,48000)):
      for layout in (6,14):
       kinds,mapping,mask=LAYOUTS[layout];widths=[2 if k==1 else 1 for k in kinds];channels=sum(widths);lfe_source=sum(widths[:kinds.index(3)]);target=mapping[lfe_source]
       noise,lfe,wrong=references[bands];offset=len(gold)
       for sample in range(len(lfe)):
        values=[0.]*channels
        for source in range(channels):values[mapping[source]]=lfe[sample] if source==lfe_source else noise[sample]*math.sqrt(2**source)
        gold.extend(struct.pack('<'+'d'*channels,*values))
       rows=[];controls=[];bad=[]
       for frame in range(6):
        audio='';extensions='';ordinary='';source=0
        for index,(kind,width) in enumerate(zip(kinds,widths)):
          core=field(index+1,4)+(cores[frame] if kind==3 else ('0'+silent*2 if kind==1 else silent));audio+=core;ordinary+=field(kind,3)+core
          if kind!=3:
            raw=sbr(list(range(source,source+width)),frame%3,len(high)-1,smoothing=False);ext=''.join(field(v,8) for v in raw);extensions+=ext
            count=len(raw);ordinary+='110'+(field(count,4) if count<15 else '1111'+field(count-14,8))+ext
          source+=width
        rows.append(store(packed(audio+extensions)));controls.append(store(packed(ordinary+'111')))
        extra=''.join(field(v,8) for v in sbr([lfe_source],frame%3,len(high)-1,smoothing=False));bad.append(store(packed(audio+extensions+extra)))
       for signal in ('explicit','sync','implicit'):
        asc=packed(field(5,5)+frequency(24000)+field(layout,4)+frequency(rate)+field(aot,5)+ga) if signal=='explicit' else packed(field(aot,5)+frequency(24000)+field(layout,4)+ga+(field(0x2b7,11)+field(5,5)+'1'+frequency(rate) if signal=='sync' else ''))
        c=dict(name=f'{aot}-{layout}-{rate}-{signal}',aot=aot,layout=layout,rate=rate,signal=signal,asc=asc.hex(),control_asc=packed(field(5,5)+frequency(24000)+field(layout,4)+frequency(rate)+field(2,5)+field(frame_samples==960,1)+'00').hex(),frames=rows,control_frames=controls,bad_frames=bad,channels=channels,channel_mask=mask,lfe_target=target,reference_offset=offset,reference_bytes=len(lfe)*channels*8,slots=frame_samples//64,bands=bands,container_rate=rate,container_frame_samples=bands*(frame_samples//32),pcm_offset=0,samples=len(lfe))
        c['video']=video_fixture([c],blob,channels=channels,filename=f'{prefix}-{c["name"]}-synthetic.mp4')
        c['bad_video']=video_fixture([dict(c,frames=[rows[0],bad[1]])],blob,channels=channels,filename=f'{prefix}-{c["name"]}-excess-synthetic.mp4')
        cases.append(c)
    (DEST/f'{prefix}-packets.bin').write_bytes(blob)
    (DEST/f'{prefix}-reference.f64le').write_bytes(gold)
    (DEST/f'{prefix}.json').write_text(json.dumps(dict(cases=cases,delay_mutant_peak={str(b):max(abs(x-y) for x,y in zip(references[b][1],references[b][2])) for b in references},provenance='Own nonzero sparse LFE core, independent direct IMDCT and QMF convolution with six-row alignment, no SBR on LFE; distinct noise SBR other channels. Own no-delay mutant demonstrates delay sensitivity. No private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')
    print(len(cases),'nonzero ER LFE SBR cases')
if __name__=='__main__':main()
