#!/usr/bin/env python3
"""Authored active ER-LTP/SBR packets; reuse own independent scalar references."""
import json,struct
from generate_aac_ltp_transition_fixtures import Oracle
from generate_aac_main_sbr_oracle import reference
from generate_aac_main_tools_fixtures import channel,ics
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def main():
    syntax=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    source=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases'] if c['slots']==15 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    blob=bytearray();rows=[];badrows=[];oracle=Oracle(960);mutant=Oracle(960);pcm=[];inactive=[]
    for frame in range(6):
        active=frame>=3;lag=960-7*(frame%3);coefficient=frame%8
        q=[[1,-1,1,-1,0,0,0,0] if frame%2 else [-1,1,-1,1,0,0,0,0]]
        pcm.extend(oracle.run(0,0,q,active,lag,coefficient,[True,False]));inactive.extend(mutant.run(0,0,q,False,lag,coefficient,[True,False]))
        info=ics(0,2,False)
        if active:info=info[:-1]+'11'+field(lag,11)+field(coefficient,3)+'10'
        core='0000'+channel(0,[1,1],q,info=info)
        # ER omits element IDs, FIL/count and END; SBR CRC is forbidden.
        row=source['frames'][2 if frame%3==1 else frame%3];raw=syntax[row['offset']:row['offset']+row['byte_length']]
        extra=''.join(field(b,8) for b in raw)
        for data,listing in ((packed(core+extra),rows),(packed(core+'1110'+extra[4:]),badrows)):
            listing.append(dict(offset=len(blob),bytes=len(data),active=active,lag=lag,coefficient=coefficient));blob.extend(data)
    cases=[]
    for rate,bands in ((24000,32),(48000,64)):
      for values,suffix in ((pcm,'reference'),(inactive,'inactive-control')):
        gold=reference(pcm_override=values,bands=bands,frame_samples=960)
        (DEST/f'aac-er-active-sbr-960-{rate}-{suffix}.f64le').write_bytes(struct.pack('<'+'d'*len(gold),*gold))
      for signal in ('explicit','sync','implicit'):
        ga='10000'
        asc=packed(field(5,5)+frequency(24000)+'0001'+frequency(rate)+field(19,5)+ga) if signal=='explicit' else packed(field(19,5)+frequency(24000)+'0001'+ga+(field(0x2b7,11)+field(5,5)+'1'+frequency(rate) if signal=='sync' else ''))
        c=dict(name=f'{rate}-{signal}',rate=rate,signal=signal,asc=asc.hex(),frames=rows,bad_frames=badrows,slots=15,bands=bands,container_rate=rate,container_frame_samples=bands*30,pcm_offset=0,samples=6*bands*30)
        c['video']=video_fixture([c],blob,filename=f'aac-er-active-sbr-960-{rate}-{signal}-synthetic.mp4')
        c['bad_video']=video_fixture([dict(c,frames=[rows[0],badrows[1]])],blob,filename=f'aac-er-active-sbr-960-{rate}-{signal}-crc-malformed-synthetic.mp4')
        cases.append(c)
    (DEST/'aac-er-active-sbr-960-packets.bin').write_bytes(blob)
    (DEST/'aac-er-active-sbr-960.json').write_text(json.dumps(dict(cases=cases,provenance='Own sparse nonzero ER LTP residuals, varying lag/coefficient and active predictor from frame 3. Own SBR no-CRC counterparts; independent scalar LTP/direct QMF gold is aac-er-active-sbr-960-{rate}-reference.f64le with inactive predictor control. No private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')
    print('six active ER-LTP SBR videos and six forbidden-CRC companions')
if __name__=='__main__':main()
