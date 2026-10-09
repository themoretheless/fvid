#!/usr/bin/env python3
"""Own late SBR over already-active LTP, scalar core/QMF and negative controls."""
import json,struct
from generate_aac_ltp_transition_fixtures import Oracle
from generate_aac_main_tools_fixtures import channel,ics
from generate_aac_main_sbr_oracle import reference
from generate_aac_pce_profile_fixtures import config,program
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
FIRST=4

def main():
    syntax=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    source=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases'] if c['slots']==16 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    core=[];inactive=[];oracle=Oracle(1024);mutant=Oracle(1024);audio=[]
    target_core=[];target_inactive=[];target_audio=[];target_oracle=Oracle(1024);target_mutant=Oracle(1024)
    for frame in range(6):
        active=frame>=3;lag=1024-7*(frame%3);coefficient=frame%8;q=[[1,-1,1,-1,0,0,0,0] if frame%2 else [-1,1,-1,1,0,0,0,0]]
        h=ics(0,2,False)
        if active:h=h[:-1]+'11'+field(lag,11)+field(coefficient,3)+'10'
        audio.append(channel(0,[1,1],q,info=h))
        core.extend(oracle.run(0,0,q,active,lag,coefficient,[True,False]));inactive.extend(mutant.run(0,0,q,False,lag,coefficient,[True,False]))
        tq=[[1,0,-1,1,1,-1,0,1] if frame%2 else [-1,0,1,-1,-1,1,0,-1]]
        th=ics(0,2,False)
        if active:th=th[:-1]+'11'+field(1007,11)+field((frame+3)%8,3)+'10'
        target_audio.append(channel(0,[1,1],tq,info=th))
        target_core.extend(target_oracle.run(0,0,tq,active,1007,(frame+3)%8,[True,False]))
        target_inactive.extend(target_mutant.run(0,0,tq,False,1007,(frame+3)%8,[True,False]))
    blob=bytearray();cases=[]
    for rate,bands in [(24000,32),(48000,64)]:
        warm=reference(pcm_override=core,bands=bands,first_sbr_frame=FIRST)
        cold=reference(pcm_override=[0.]*(FIRST*1024)+core[FIRST*1024:],bands=bands,first_sbr_frame=FIRST)
        wrong=reference(pcm_override=inactive,bands=bands,first_sbr_frame=FIRST)
        normal=core if rate==24000 else reference(pcm_override=core,bands=bands,sbr_frames=[False]*6)
        if rate==24000:warm=core[:FIRST*1024]+warm[FIRST*1024:];wrong=inactive[:FIRST*1024]+wrong[FIRST*1024:]
        def f32(value):return struct.unpack('<f',struct.pack('<f',value))[0]
        def add(left,right):return [f32(f32(a)+f32(b)) for a,b in zip(left,right)]
        target_qmf=reference(pcm_override=target_core,bands=bands,sbr_frames=[False]*6)
        target_wrong=reference(pcm_override=target_inactive,bands=bands,sbr_frames=[False]*6)
        target_cold=reference(pcm_override=[0.]*(FIRST*1024)+target_core[FIRST*1024:],bands=bands,sbr_frames=[False]*6)
        if rate==24000:
            target_qmf=target_core[:FIRST*1024]+target_qmf[FIRST*1024:]
            target_wrong=target_inactive[:FIRST*1024]+target_wrong[FIRST*1024:]
        for program_name in ('sce','cce','cce-active'):
            coupled=program_name!='sce';active_target=program_name=='cce-active'
            prefix=f'aac-ltp-late-sbr-{rate}'+('-mixed' if active_target else '')
            values=[('reference',add(warm,target_qmf) if active_target else warm),
                    ('cold-qmf-control',add(cold,target_cold) if active_target else cold),
                    ('inactive-ltp-control',add(wrong,target_wrong) if active_target else wrong),
                    ('core-reference',add(normal,target_core if rate==24000 else reference(pcm_override=target_core,bands=bands,sbr_frames=[False]*6)) if active_target else normal)]
            for label,data in values:
                (DEST/f'{prefix}-{label}.f64le').write_bytes(struct.pack('<'+str(len(data))+'d',*data))
            for late in (False,True):
                rows=[];adts=bytearray();bad_packet=None
                asc=config(4,1,coupling=((True,1),)) if coupled else packed(field(4,5)+frequency(24000)+'0001'+'000')
                for frame in range(6):
                    target='0000000'+(target_audio[frame] if active_target else channel(0,[0,0],[[0]*8],info=ics(0,2,False))) if coupled else '0000000'+audio[frame]
                    source_audio='0100001'+'1'+'000'+'0'+'0000'+'0'+'0'+'10'+audio[frame] if coupled else ''
                    fill=''
                    if late and frame>=FIRST:
                        r=source['frames'][(frame-FIRST)%3];data=syntax[r['offset']:r['offset']+r['byte_length']]
                        fill='110'+(field(len(data),4) if len(data)<15 else '1111'+field(len(data)-14,8))+''.join(field(b,8) for b in data)
                    raw=target+source_audio+fill+'111';packet=packed(raw)
                    if late and frame==5:
                        corrupt=bytearray(data);corrupt[0]^=1
                        bad_fill=fill[:-len(data)*8]+''.join(field(b,8) for b in corrupt)
                        bad_packet=packed(target+source_audio+bad_fill+'111')
                    rows.append(dict(offset=len(blob),bytes=len(packet),active_ltp=frame>=3,sbr=late and frame>=FIRST));blob.extend(packet)
                    # PCE config0 is explicitly bootstrapped in each raw block.
                    transport_packet=packed(program('101',4,1,coupling=((True,1),))+raw) if coupled else packet
                    header=packed(field(0xfff,12)+'0'+'00'+'1'+'11'+frequency(24000)+'0'+field(0 if coupled else 1,3)+'0000'+field(len(transport_packet)+7,13)+field(0x7ff,11)+'00')
                    adts.extend(header+transport_packet)
                name=str(rate)+'-'+program_name+('-late' if late else '-core')
                c=dict(name=name,asc=asc.hex(),rate=rate,late=late,coupled=coupled,program=program_name,frames=rows,container_rate=rate,container_frame_samples=1024*(rate//24000),slots=16,bands=bands,samples=len(normal),pcm_offset=0,reference=prefix+'-'+('reference' if late else 'core-reference')+'.f64le',cold_control=prefix+'-cold-qmf-control.f64le',inactive_control=prefix+'-inactive-ltp-control.f64le')
                c['video']=video_fixture([c],blob,filename=f'aac-ltp-late-sbr-{name}-synthetic.mp4')
                if rate==48000 and late:
                    file=f'aac-ltp-late-sbr-{program_name}-synthetic.aac';(DEST/file).write_bytes(adts);c['adts']=file
                if bad_packet is not None:
                    file=f'aac-ltp-late-sbr-{name}-bad-crc.bin';(DEST/file).write_bytes(bad_packet);c['bad_packet']=file
                    bad_rows=rows[:5]+[dict(rows[5],offset=len(blob),bytes=len(bad_packet))];blob.extend(bad_packet)
                    c['bad_video']=video_fixture([dict(c,frames=bad_rows)],blob,filename=f'aac-ltp-late-sbr-{name}-bad-crc-synthetic.mp4')
                cases.append(c)
    (DEST/'aac-ltp-late-sbr-packets.bin').write_bytes(blob)
    (DEST/'aac-ltp-late-sbr.json').write_text(json.dumps(dict(cases=cases,first_sbr=FIRST,provenance='Own mono and independent CCE LTP active before first FIL, direct float LTP history/cosines and scalar QMF. Cold-QMF and inactive-LTP controls. No private media, foreign decoder or network.'),indent=2)+'\n')
    print('twelve late/core LTP videos, six CRC videos/packets, three ADTS transports, sixteen scalar references/controls')
if __name__=='__main__':main()
