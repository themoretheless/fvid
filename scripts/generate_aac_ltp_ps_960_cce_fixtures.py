#!/usr/bin/env python3
"""960 LTP/TNS/CCE scalar core and PS composition fixtures, offline only."""
import struct
from generate_aac_ltp_phase_fixtures import Oracle,metadata,info,f32
from generate_aac_main_tools_fixtures import channel
from generate_aac_pce_profile_fixtures import program
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def generate(blob,payloads):
    cases=[]
    for point in (0,1,3):
        target=Oracle(960);source=Oracle(960);inactive=Oracle(960)
        rows=[];core=bytearray();source_core=bytearray();inactive_core=bytearray()
        for frame in range(12):
            td=metadata(frame,0,point,960);sd=metadata(frame,1,point,960)
            tq=[1,-1,1,0,0,1,-1,1];sq=[-1 if (frame+k)%3==0 else 1 if (frame+k)%3==1 else 0 for k in range(8)]
            prepared=source.prepare(sq,sd)
            residual=[f32(v*1024.+s)/1024. for v,s in zip(tq,prepared)] if point==0 else tq
            spectrum=target.prepare(residual,td)
            if point==1:spectrum=[f32(a+b) for a,b in zip(spectrum,prepared)]
            pcm=target.synthesize(spectrum,td)
            if point==3:
                source_core.extend(struct.pack('<960f',*source.synthesize(prepared,sd)))
                disabled=dict(sd,active=False)
                inactive_core.extend(struct.pack('<960f',*inactive.synthesize(inactive.prepare(sq,disabled),disabled)))
            core.extend(struct.pack('<960f',*pcm))
            audio='0000000'+channel(0,[1,1],[tq],info=info(td),tns=(td['reverse'],1))
            cce='010'+field(1,4)+field(point==3,1)+'000'+'0'+'0000'+field(point==1,1)+'0'+'10'+channel(0,[1,1],[sq],info=info(sd),tns=(sd['reverse'],1))
            payload=bytes.fromhex(payloads[frame%3]);ps=fill(payload)
            wire=cce+audio+ps if frame%2 else audio+ps+cce
            packet=packed(wire+'111');row=dict(offset=len(blob),bytes=len(packet),payload=payload.hex(),target=td,source=sd);blob.extend(packet)
            if payload[0]>>4==14:
                corrupt=bytearray(payload);corrupt[0]^=1;bad_ps=fill(bytes(corrupt))
                bad=packed((cce+audio+bad_ps if frame%2 else audio+bad_ps+cce)+'111')
                row.update(bad_offset=len(blob),bad_bytes=len(bad));blob.extend(bad)
            if point==3 and frame==7:
                bad=bytearray(packet);bad[1]=(bad[1]&0xf0)|1
                row.update(absent_offset=len(blob),absent_bytes=len(bad));blob.extend(bad)
            rows.append(row)
        core_file=f'aac-ltp-ps-960-cce-{point}-core.f32le';(DEST/core_file).write_bytes(core)
        source_file=f'aac-ltp-ps-960-cce-{point}-source-core.f32le'
        if point==3:
            (DEST/source_file).write_bytes(source_core)
            (DEST/'aac-ltp-ps-960-cce-3-inactive-source-core.f32le').write_bytes(inactive_core)
        for rate in (24000,48000):
            for signal in ('explicit','sync','implicit'):
                prefix=field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(4,5)+'100' if signal=='explicit' else field(4,5)+frequency(24000)+'0000'+'100'
                prefix=program(prefix,4,1,coupling=((point==3,1),))
                if signal=='sync':prefix+=field(0x2b7,11)+field(5,5)+'1'+frequency(rate)+field(0x548,11)+'1'
                c=dict(name=f'cce-{point}-{rate}-{signal}',point=point,core=core_file,source_core=source_file if point==3 else None,asc=packed(prefix).hex(),signal=signal,frames=rows,slots=15,bands=32*(rate//24000),container_rate=rate,container_frame_samples=960*(rate//24000),channels=2,samples=11520*(rate//24000),pcm_offset=0)
                c['video']=video_fixture([c],blob,channels=2,filename=f'aac-ltp-ps-960-{c["name"]}-synthetic.mp4')
                if rate==48000 and signal=='explicit':
                    bad_rows=rows[:7]+[dict(rows[7],offset=rows[7]['bad_offset'],bytes=rows[7]['bad_bytes'])]
                    c['bad_video']=video_fixture([dict(c,frames=bad_rows)],blob,channels=2,filename=f'aac-ltp-ps-960-cce-{point}-bad-crc-synthetic.mp4')
                    if point==3:
                        absent_rows=rows[:7]+[dict(rows[7],offset=rows[7]['absent_offset'],bytes=rows[7]['absent_bytes'])]
                        c['absent_video']=video_fixture([dict(c,frames=absent_rows)],blob,channels=2,filename='aac-ltp-ps-960-cce-3-absent-target-synthetic.mp4')
                cases.append(c)
    return cases
