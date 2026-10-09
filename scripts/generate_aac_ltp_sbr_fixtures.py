#!/usr/bin/env python3
"""Own active LTP/SBR packets and direct scalar PCM, no foreign codec."""
import json,struct
from generate_aac_ltp_transition_fixtures import Oracle
from generate_aac_main_tools_fixtures import channel,ics
from generate_aac_main_sbr_oracle import reference
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def main():
    syntax=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    source=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases'] if c['slots']==16 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    blob=bytearray();rows=[];core=[];control=[];oracle=Oracle(1024);mutant=Oracle(1024);adts=bytearray();bad=None
    for frame in range(6):
        active=frame>=3;lag=1024-7*(frame%3);coefficient=frame%8;used=[True,False]
        q=[[1,-1,1,-1,0,0,0,0] if frame%2 else [-1,1,-1,1,0,0,0,0]]
        info=ics(0,2,False)
        if active:info=info[:-1]+'11'+field(lag,11)+field(coefficient,3)+'10'
        audio='0000000'+channel(0,[1,1],q,info=info)
        row=source['frames'][frame%3];raw=syntax[row['offset']:row['offset']+row['byte_length']]
        fill='110'+(field(len(raw),4) if len(raw)<15 else '1111'+field(len(raw)-14,8))
        packet=packed(audio+fill+''.join(field(b,8) for b in raw)+'111')
        if frame==1:
            corrupt=bytearray(raw);corrupt[0]^=1
            bad=packed(audio+fill+''.join(field(b,8) for b in corrupt)+'111')
        rows.append(dict(offset=len(blob),bytes=len(packet),active=active,lag=lag,coefficient=coefficient));blob.extend(packet)
        core.extend(oracle.run(0,0,q,active,lag,coefficient,used));control.extend(mutant.run(0,0,q,False,lag,coefficient,used))
        header=packed(field(0xfff,12)+'0'+'00'+'1'+'11'+frequency(24000)+'0'+'001'+'0000'+field(len(packet)+7,13)+field(0x7ff,11)+'00')
        assert len(header)==7;adts.extend(header+packet)
    cases=[]
    for rate,bands in [(24000,32),(48000,64)]:
        gold=reference(pcm_override=core,bands=bands);wrong=reference(pcm_override=control,bands=bands)
        (DEST/f'aac-ltp-sbr-{rate}-reference.f64le').write_bytes(struct.pack('<'+str(len(gold))+'d',*gold))
        (DEST/f'aac-ltp-sbr-{rate}-inactive-control.f64le').write_bytes(struct.pack('<'+str(len(wrong))+'d',*wrong))
        for signal in ('explicit','sync','implicit'):
            asc=packed(field(5,5)+frequency(24000)+'0001'+frequency(rate)+field(4,5)+'000') if signal=='explicit' else packed(field(4,5)+frequency(24000)+'0001'+'000'+(field(0x2b7,11)+field(5,5)+'1'+frequency(rate) if signal=='sync' else ''))
            case=dict(rate=rate,signal=signal,asc=asc.hex(),frames=rows,slots=16,bands=bands,container_rate=rate,container_frame_samples=bands*32,pcm_offset=0,samples=len(gold))
            case['video']=video_fixture([case],blob,filename=f'aac-ltp-sbr-{rate}-{signal}-synthetic.mp4');cases.append(case)
    (DEST/'aac-ltp-sbr-bad-crc.bin').write_bytes(bad)
    first=bytes(blob[:rows[0]['bytes']]);bad_rows=[dict(rows[0],offset=0),dict(rows[1],offset=len(first),bytes=len(bad))]
    video_fixture([dict(cases[2],frames=bad_rows)],first+bad,filename='aac-ltp-sbr-bad-crc-synthetic.mp4')
    (DEST/'aac-ltp-sbr-packets.bin').write_bytes(blob)
    (DEST/'aac-ltp-sbr-synthetic.aac').write_bytes(adts)
    (DEST/'aac-ltp-sbr.json').write_text(json.dumps(dict(cases=cases,provenance='Own active LTP sparse direct transforms/float history followed by direct SBR QMF convolutions; explicit/sync/implicit clocks 24/48 kHz, ADTS AOT4. No private media or foreign codec.'),indent=2)+'\n')
    print('generated six LTP/SBR acceptance videos, CRC video, ADTS and scalar active/inactive PCM')
if __name__=='__main__':main()
