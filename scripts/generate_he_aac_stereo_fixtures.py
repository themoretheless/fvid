#!/usr/bin/env python3
"""Original silent LC CPE + centered coupled/uncoupled SBR. Offline only.
Each channel has the original mono oracle's envelope/noise energies.
No encoder, decoder, private media, FFmpeg or network.
"""
from pathlib import Path
import json,hashlib
from generate_he_aac_packet_fixtures import DEST,field,packed,video_fixture
from generate_aac_sbr_data_fixtures import word
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_sbr_extension_fixtures import crc
from generate_aac_sbr_frequency_oracles import tables

def payload(nhigh,mode,smoothing,frame,coupled):
    temporal=frame>0
    env=word(0,0)*nhigh if temporal else field(2,7)+word(1,0)*(nhigh-1)
    noise=word(8,0) if temporal else field(7,5)
    balance_env=word(2,0)*nhigh if temporal else field(12,6)+word(3,0)*(nhigh-1)
    balance_noise=word(9,0) if temporal else field(6,5)
    data='0'+field(coupled,1)+'00001'*(1 if coupled else 2)+field(temporal,1)*4+'00'*(1 if coupled else 2)
    data+=(env+noise+balance_env+balance_noise) if coupled else (env+env+noise+noise)
    data+='000' # no harmonics in either channel; no extension data
    body=field(frame==0,1)+(header(mode,smoothing) if frame==0 else '')+data
    protected=frame==1
    body+='0'*(-(4+10*protected+len(body))%8)
    return packed(field(14 if protected else 13,4)+(field(crc(body),10) if protected else '')+body)

def packet(raw):
    channel=field(100,8)+'0'+'00'+'0'+'000000'+'0'+'000'
    core='001'+'0000'+'0'+channel*2 # CPE tag0, independent windows
    count=len(raw)
    fill='110'+(field(count,4) if count<15 else '1111'+field(count-14,8))
    crc_bit=len(core)+len(fill)+4
    return packed(core+fill+''.join(field(b,8) for b in raw)+'111'),crc_bit

def main():
    source=json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())
    _,high,_,_=tables(10,27,0,False,0,0)
    blob=bytearray();cases=[]
    for ref in source['cases']:
        for coupled in [False,True]:
            out=48000 if ref['bands']==64 else 24000
            rate_index=3 if out==48000 else 6
            asc=packed(field(5,5)+field(6,4)+'0010'+field(rate_index,4)+field(2,5)+field(ref['slots']==15,1)+'00')
            frames=[]
            for frame in range(3):
                raw=payload(len(high)-1,ref['limiter'],ref['smoothing'],frame,coupled)
                data,bit=packet(raw)
                frames.append(dict(offset=len(blob),bytes=len(data),crc_bit=bit));blob.extend(data)
            cases.append(dict(slots=ref['slots'],bands=ref['bands'],coupled=coupled,signalling='explicit',asc=asc.hex(),frames=frames,pcm_offset=ref['pcm_offset'],samples=ref['samples']))
    (DEST/'he-aac-sbr-stereo.bin').write_bytes(blob)
    video=video_fixture(cases,blob,2,'he-aac-sbr-stereo.mp4')
    (DEST/'he-aac-sbr-stereo.json').write_text(json.dumps(dict(kind='original centered stereo complete LC CPE + SBR',sha256=hashlib.sha256(blob).hexdigest(),video=video,cases=cases),separators=(',',':'))+'\n')
    print(len(cases),'three-frame stereo sequences;',len(blob),'bytes')
if __name__=='__main__':main()
