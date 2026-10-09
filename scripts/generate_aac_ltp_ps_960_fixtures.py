#!/usr/bin/env python3
"""Own 960 LTP transitions with authored 30-slot PS; no external codec."""
import json
from generate_aac_main_tools_fixtures import channel,ics
from generate_aac_ltp_transition_fixtures import SEQUENCES
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def main():
    transition=json.loads((DEST/'aac-ltp-transitions.json').read_text())['cases'][0]
    old=(DEST/'aac-ltp-transitions-packets.bin').read_bytes()
    gold=(DEST/'aac-ltp-transitions-reference.f32le').read_bytes()
    payloads=json.loads((DEST/'aac-sbr-ps-30-oracles.json').read_text())['videos'][0]['sbr_payloads']
    blob=bytearray();core=bytearray();rows=[];cases=[]
    for frame,seq in enumerate(SEQUENCES):
        active=seq!=2 and frame>=1;lag=960-13*(frame%3);coefficient=frame%8;used=[True,frame%3!=1]
        q=[[(-1 if (frame+w+k)%3==0 else 1 if (frame+w+k)%3==1 else 0) for k in range(8)] for w in range(8 if seq==2 else 1)]
        info=ics(seq,2,False,shape=frame%2)
        if active:info=info[:-1]+'11'+field(lag,11)+field(coefficient,3)+''.join(str(int(v)) for v in used)
        audio='0000000'+channel(seq,[1,1],q,info=info)
        previous=transition['frames'][frame]
        assert packed(audio+'111')==old[previous['offset']:previous['offset']+previous['bytes']]
        core.extend(gold[previous['reference_offset']:previous['reference_offset']+960*4])
        payload=bytes.fromhex(payloads[frame%3]);packet=packed(audio+fill(payload)+'111')
        row=dict(offset=len(blob),bytes=len(packet),payload=payload.hex());blob.extend(packet)
        if payload[0]>>4==14:
            corrupt=bytearray(payload);corrupt[0]^=1;bad=packed(audio+fill(bytes(corrupt))+'111')
            row.update(bad_offset=len(blob),bad_bytes=len(bad));blob.extend(bad)
        rows.append(row)
    for rate in (24000,48000):
        for signal in ('explicit','sync','implicit'):
            asc=field(29,5)+frequency(24000)+'0001'+frequency(rate)+field(4,5)+'100' if signal=='explicit' else field(4,5)+frequency(24000)+'0001'+'100'
            if signal=='sync':asc+=field(0x2b7,11)+field(5,5)+'1'+frequency(rate)+field(0x548,11)+'1'
            c=dict(name=f'{rate}-{signal}',asc=packed(asc).hex(),signal=signal,frames=rows,slots=15,bands=32*(rate//24000),container_rate=rate,container_frame_samples=960*(rate//24000),channels=2,samples=11520*(rate//24000),pcm_offset=0)
            c['video']=video_fixture([c],blob,channels=2,filename=f'aac-ltp-ps-960-{c["name"]}-synthetic.mp4')
            if rate==48000 and signal=='explicit':
                bad_rows=rows[:7]+[dict(rows[7],offset=rows[7]['bad_offset'],bytes=rows[7]['bad_bytes'])]
                c['bad_video']=video_fixture([dict(c,frames=bad_rows)],blob,channels=2,filename='aac-ltp-ps-960-bad-crc-synthetic.mp4')
            cases.append(c)
    from generate_aac_ltp_ps_960_cce_fixtures import generate
    cases.extend(generate(blob,payloads))
    (DEST/'aac-ltp-ps-960-core.f32le').write_bytes(core)
    (DEST/'aac-ltp-ps-960-packets.bin').write_bytes(blob)
    (DEST/'aac-ltp-ps-960.json').write_text(json.dumps(dict(cases=cases,provenance='Own 960 LTP transitions and CCE point0/1/3 with scalar core/source histories; authored 30-slot PS payloads. No private media, external codec, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
