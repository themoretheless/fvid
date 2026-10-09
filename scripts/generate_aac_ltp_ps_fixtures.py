#!/usr/bin/env python3
"""Owned LTP core/CCE packets plus authored PS; no private media or codec process."""
import json,struct
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
from generate_aac_main_tools_fixtures import channel,ics
from generate_aac_ltp_transition_fixtures import SEQUENCES,Oracle
from generate_aac_ltp_phase_fixtures import metadata,info,Oracle as PhaseOracle
from generate_aac_pce_profile_fixtures import program
from generate_aac_ps_coupling_fixtures import fill

def main():
    payloads=json.loads((DEST/'aac-ssr-ps.json').read_text())['payloads'];blob=bytearray();cases=[]
    transition=json.loads((DEST/'aac-ltp-transitions.json').read_text())['cases'][1]
    old_transition=(DEST/'aac-ltp-transitions-packets.bin').read_bytes();old_phase=(DEST/'aac-ltp-phase-packets.bin').read_bytes()
    phase=json.loads((DEST/'aac-ltp-phase.json').read_text())['cases'];phase_gold=(DEST/'aac-ltp-phase-reference.f32le').read_bytes();control_gold=(DEST/'aac-ltp-phase-control-reference.f32le').read_bytes()
    for kind,point in [('mono',None),('cce-0',0),('cce-1',1),('cce-3',3)]:
        rows=[];adts=bytearray();core=bytearray();source_core=bytearray();source_oracle=PhaseOracle()
        for frame in range(12):
            if point is None:
                seq=SEQUENCES[frame];shape=frame%2;active=seq!=2 and frame>=1;lag=1024-13*(frame%3);coefficient=frame%8;used=[True,frame%3!=1]
                q=[[(-1 if (frame+w+k)%3==0 else 1 if (frame+w+k)%3==1 else 0) for k in range(8)] for w in range(8 if seq==2 else 1)]
                h=ics(seq,2,False,shape=shape)
                if active:h=h[:-1]+'11'+field(lag,11)+field(coefficient,3)+''.join(str(int(v)) for v in used)
                audio='0000000'+channel(seq,[1,1],q,info=h);cce='';raw=audio
                r=transition['frames'][frame];assert packed(raw+'111')==old_transition[r['offset']:r['offset']+r['bytes']]
                gold=(DEST/'aac-ltp-transitions-reference.f32le').read_bytes();core+=gold[r['reference_offset']:r['reference_offset']+4096]
            else:
                td=metadata(frame,0,point);sd=metadata(frame,1,point);tq=[1,-1,1,0,0,1,-1,1];sq=[-1 if (frame+k)%3==0 else 1 if (frame+k)%3==1 else 0 for k in range(8)]
                audio='0000000'+channel(0,[1,1],[tq],info=info(td),tns=(td['reverse'],1))
                cce='010'+field(1,4)+field(point==3,1)+'000'+'0'+'0000'+field(point==1,1)+'0'+'10'+channel(0,[1,1],[sq],info=info(sd),tns=(sd['reverse'],1))
                raw=cce+audio if frame%2 else audio+cce;r=phase[2 if point==3 else point]['frames'][frame]
                assert packed(raw+'111')==old_phase[r['offset']:r['offset']+r['bytes']]
                at=r['control_reference_offset'] if point==3 else r['reference_offset'];gold=control_gold if point==3 else phase_gold;core+=gold[at:at+4096]
                if point==3:source_core+=struct.pack('<1024f',*source_oracle.synthesize(source_oracle.prepare(sq,sd),sd))
            payload=bytes.fromhex(payloads[frame%3]);ps=fill(payload)
            wire=cce+audio+ps if point is not None and frame%2 else audio+ps+cce
            packet=packed(wire+'111');row=dict(offset=len(blob),bytes=len(packet),payload=payload.hex());rows.append(row);blob.extend(packet)
            if payload[0]>>4==14:
                corrupt=bytearray(payload);corrupt[0]^=1;bad_fill=fill(bytes(corrupt))
                bad_wire=cce+audio+bad_fill if point is not None and frame%2 else audio+bad_fill+cce
                bad=packed(bad_wire+'111');row['bad_offset']=len(blob);row['bad_bytes']=len(bad);blob.extend(bad)
            transport=packed(program('101',4,1,coupling=((point==3,1),))+wire+'111') if point is not None else packet
            header=packed(field(0xfff,12)+'0'+'00'+'1'+'11'+frequency(24000)+'0'+field(0 if point is not None else 1,3)+'0000'+field(len(transport)+7,13)+field(0x7ff,11)+'00');adts.extend(header+transport)
        core_file=f'aac-ltp-ps-{kind}-core.f32le';(DEST/core_file).write_bytes(core)
        if source_core:(DEST/'aac-ltp-ps-cce-3-source-core.f32le').write_bytes(source_core)
        adts_file=f'aac-ltp-ps-{kind}-synthetic.aac';(DEST/adts_file).write_bytes(adts)
        for rate in (24000,48000):
            for signal in ('explicit','sync','implicit'):
                prefix=field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(4,5)+'000' if signal=='explicit' else field(4,5)+frequency(24000)+'0000'+'000'
                if point is None:prefix=prefix[:9]+'0001'+prefix[13:]
                else:prefix=program(prefix,4,1,coupling=((point==3,1),))
                if signal=='sync':prefix+=field(0x2b7,11)+field(5,5)+'1'+frequency(rate)+field(0x548,11)+'1'
                c=dict(name=f'{kind}-{rate}-{signal}',kind=kind,point=point,signal=signal,asc=packed(prefix).hex(),frames=rows,core=core_file,adts=adts_file,channels=2,slots=16,bands=32*(rate//24000),container_rate=rate,container_frame_samples=1024*(rate//24000),samples=12288*(rate//24000),pcm_offset=0)
                c['video']=video_fixture([c],blob,channels=2,filename=f'aac-ltp-ps-{c["name"]}-synthetic.mp4')
                if rate==48000 and signal=='explicit':
                    bad_rows=rows[:7]+[dict(rows[7],offset=rows[7]['bad_offset'],bytes=rows[7]['bad_bytes'])]
                    c['bad_video']=video_fixture([dict(c,frames=bad_rows)],blob,channels=2,filename=f'aac-ltp-ps-{kind}-bad-crc-synthetic.mp4')
                cases.append(c)
    (DEST/'aac-ltp-ps-packets.bin').write_bytes(blob)
    (DEST/'aac-ltp-ps.json').write_text(json.dumps(dict(cases=cases,provenance='Own qualified LTP transition/CCE bit writers and scalar core PCM, with previously authored PS payloads. CCE points 0/1/3, wire order alternation. No private media or foreign codec.'),indent=2)+'\n')
    print('24 original LTP/PS videos, four ADTS streams, scalar core references')
if __name__=='__main__':main()
