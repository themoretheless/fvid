#!/usr/bin/env python3
"""Authored LTP ADTS boundaries/CRC; pure bit writers, no codec process or network."""
import json
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture
from generate_aac_main_tools_fixtures import channel, ics
from generate_adts_multiblock_fixtures import transport
from generate_aac_pce_profile_fixtures import program, config
from generate_aac_ltp_phase_fixtures import metadata, info

def region(start, end, width=192):
    return dict(start=start, end=min(end,start+width), width=width)

def main():
    blob=bytearray();cases=[];invalid=[]
    def save(name, packets, spans, asc, channels, rate=24000):
        rows=[]
        for packet, regions in zip(packets,spans):
            header=packed(field(0xfff,12)+'0'+'00'+'1'+'11'+frequency(24000)+'0'+field(channels if name[:3]!='cce' else 0,3)+'0000'+field(len(packet)+7,13)+field(0x7ff,11)+'00')
            rows.append(dict(offset=len(blob),bytes=len(packet),regions=regions,header=header.hex()));blob.extend(packet)
        c=dict(name=name,asc=asc,frames=rows,channels=channels,container_rate=rate,container_frame_samples=1024*(rate//24000),slots=16,bands=32*(rate//24000),samples=len(rows)*1024*(rate//24000),pcm_offset=0)
        c['video']=video_fixture([c],blob,channels=channels,filename=f'adts-ltp-{name}-synthetic.mp4');c['transports']=[]
        for counts in ([1]*len(rows),[2]*(len(rows)//2),[3]*(len(rows)//3),[4]*(len(rows)//4),[4,2] if len(rows)==6 else [1,2,1,4,4]):
            if sum(counts)!=len(rows):continue
            for crc in (False,True):
                data=bytearray();at=0
                for n in counts:data.extend(transport(rows[at:at+n],blob,crc));at+=n
                label=str(counts[0]) if len(set(counts))==1 else 'mixed'
                file=f'adts-ltp-{name}-{label}'+('-crc' if crc else '')+'-synthetic.aac'
                (DEST/file).write_bytes(data);c['transports'].append(dict(file=file,crc=crc,blocks=counts[0],counts=counts))
        protected=[transport([row],blob,True) for row in rows]
        bad=bytearray(b''.join(protected));at=sum(map(len,protected[:3]));bad[at+7]^=1
        file=f'adts-ltp-{name}-bad-active-crc-synthetic.aac';(DEST/file).write_bytes(bad)
        invalid.append(dict(file=file,warm_blocks=3,error='ADTS CRC mismatch'))
        cases.append(c)
    original=(DEST/'aac-ltp-pair-packets.bin').read_bytes()
    pair_manifest=json.loads((DEST/'aac-ltp-pair.json').read_text())['cases']
    for source in pair_manifest:
        name=source['name'];common=name!='independent';mode=int(name[-1]) if common else 0;packets=[];spans=[]
        for frame,row in enumerate(source['frames']):
            active=[frame>=3 and frame%4 in (0,1,3),frame>=3 and frame%4 in (0,2,3)]
            def predictor(ch):return field(1024-ch*17,11)+field((frame+ch*3)%8,3)+'11'
            def header(ch):
                base=ics(0,2,False,shape=frame%2 if common else (frame+ch)%2)
                return base[:-1]+'11'+predictor(ch) if active[ch] else base
            if common:
                h=ics(0,2,False,shape=frame%2)
                if any(active):h=h[:-1]+'1'+''.join(str(int(active[ch]))+(predictor(ch) if active[ch] else '') for ch in (0,1))
                prefix='1'+h+field(mode,2)+('10' if mode==1 else '')
            else:prefix='0'
            left='0010000'+prefix+channel(0,[1,1],[[1,-1,1,-1,0,1,-1,1]],info='' if common else header(0))
            raw=left+channel(0,[1,1],[[-1,0,1,1,-1,1,0,-1]],info='' if common else header(1))
            packet=packed(raw+'111');assert packet==original[row['offset']:row['offset']+row['bytes']]
            packets.append(packet);spans.append([region(3,len(raw)),region(len(left),len(raw),128)])
        save('pair-'+name,packets,spans,source['asc'],2)
    syntax=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    source=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases'] if c['slots']==16 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    original=(DEST/'aac-ltp-sbr-packets.bin').read_bytes();original_rows=json.loads((DEST/'aac-ltp-sbr.json').read_text())['cases'][0]['frames'];packets=[];spans=[]
    for frame in range(6):
        h=ics(0,2,False)
        if frame>=3:h=h[:-1]+'11'+field(1024-7*(frame%3),11)+field(frame%8,3)+'10'
        audio='0000000'+channel(0,[1,1],[[1,-1,1,-1,0,0,0,0] if frame%2 else [-1,1,-1,1,0,0,0,0]],info=h)
        row=source['frames'][frame%3];raw=syntax[row['offset']:row['offset']+row['byte_length']]
        fill='110'+(field(len(raw),4) if len(raw)<15 else '1111'+field(len(raw)-14,8))
        packet=packed(audio+fill+''.join(field(b,8) for b in raw)+'111');r=original_rows[frame];assert packet==original[r['offset']:r['offset']+r['bytes']]
        packets.append(packet);spans.append([region(3,len(audio))])
    save('mono-sbr',packets,spans,packed(field(4,5)+frequency(24000)+'0001'+'000').hex(),1,48000)
    for point in (0,1,3):
        packets=[];spans=[]
        for frame in range(12):
            td=metadata(frame,0,point);sd=metadata(frame,1,point)
            audio='0000000'+channel(0,[1,1],[[1,-1,1,0,0,1,-1,1]],info=info(td),tns=(td['reverse'],1))
            sq=[-1 if (frame+k)%3==0 else 1 if (frame+k)%3==1 else 0 for k in range(8)]
            cce='010'+field(1,4)+field(point==3,1)+'000'+'0'+'0000'+field(point==1,1)+'0'+'10'+channel(0,[1,1],[sq],info=info(sd),tns=(sd['reverse'],1))
            raw=program('101',4,1,coupling=((point==3,1),));regions=[region(3,len(raw),len(raw)-3)]
            for element in ([cce,audio] if frame%2 else [audio,cce]):
                start=len(raw)+3;raw+=element;regions.append(region(start,len(raw)))
            packets.append(packed(raw+'111'));spans.append(regions)
        save('cce-'+str(point),packets,spans,config(4,1,coupling=((point==3,1),)).hex(),1)
    (DEST/'adts-ltp-packets.bin').write_bytes(blob)
    (DEST/'adts-ltp.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,provenance='Own bit-authored active AOT4 SCE/SBR, CPE common/independent, PCE/CCE points 0/1/3. CRC uses independent GF(2) division and writer-measured spans, never decoder-discovered regions.'),indent=2)+'\n')
    print(len(cases),'LTP companion videos and',sum(len(c['transports']) for c in cases),'ADTS transports')
if __name__=='__main__':main()
