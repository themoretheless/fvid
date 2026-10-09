#!/usr/bin/env python3
"""Original independent SSR window drift cases with scalar PCM; offline only."""
import json,hashlib,struct
from generate_aac_ssr_fixtures import DEST,field,frequency,packed,video_fixture,SAMPLES,oracle,channel,spectrum
from generate_aac_ssr_coupling_fixtures import packet,config,silent

def matroska_fixture(case,blob):
    from generate_aac_ps_matroska_fixtures import element,number,child
    original=(DEST/'avc-slice-lists-temporal.mp4').read_bytes()
    track=child(child(original,b'moov'),b'trak')
    stbl=child(child(child(track,b'mdia'),b'minf'),b'stbl')
    entry=child(child(stbl,b'stsd')[8:],b'avc1');avcc=child(entry[78:],b'avcC')
    width,height=struct.unpack_from('>HH',entry,24)
    sizes=child(stbl,b'stsz');size=struct.unpack_from('>I',sizes,4)[0] or struct.unpack_from('>I',sizes,12)[0]
    offset=struct.unpack_from('>I',child(stbl,b'stco'),8)[0];picture=original[offset:offset+size]
    audio=element(0xe1,element(0xb5,struct.pack('>d',24000))+number(0x9f,case['channels']))
    atrack=element(0xae,number(0xd7,1)+number(0x73c5,1)+number(0x83,2)+element(0x86,b'A_AAC')+element(0x63a2,bytes.fromhex(case['asc']))+number(0x23e383,42666667)+audio)
    vtrack=element(0xae,number(0xd7,2)+number(0x73c5,2)+number(0x83,1)+element(0x86,b'V_MPEG4/ISO/AVC')+element(0x63a2,avcc)+element(0xe0,number(0xb0,width)+number(0xba,height)))
    clusters=[]
    for i,row in enumerate(case['frames']):
        cluster=number(0xe7,(i*1024*1000000+12000)//24000)
        if i==0:cluster+=element(0xa3,b'\x82'+struct.pack('>hB',0,0x80)+picture)
        packet=blob[row['offset']:row['offset']+row['bytes']]
        cluster+=element(0xa3,b'\x81'+struct.pack('>hB',0,0x80)+packet)
        clusters.append(element(0x1f43b675,cluster))
    header=element(0x1a45dfa3,element(0x4282,b'matroska')+number(0x4287,4)+number(0x4285,2))
    info=element(0x1549a966,number(0x2ad7b1,1000)+element(0x4489,struct.pack('>d',256000)))
    data=header+element(0x18538067,info+element(0x1654ae6b,atrack+vtrack)+b''.join(clusters))
    name='aac-ssr-alignment-'+case['name']+'-synthetic.mkv';(DEST/name).write_bytes(data)
    return dict(file=name,sha256=hashlib.sha256(data).hexdigest())

def main():
    cases=[];blob=bytearray();gold=bytearray()
    for name,target,source in [
        ('source-ahead',[0]*6,[0,1,2,3,0,0]),
        ('source-starts-ahead',[0]*6,[1,2,3,0,0,0]),
        ('source-behind',[0,1,2,3,0,0],[0]*6),
        ('late-source-behind',[0,0,0,1,2,3],[0]*6),
        ('opposite-switches',[0,1,2,3,0,0],[2,3,0,1,2,2]),
    ]:
        shapes=[i%2 for i in range(6)]
        pcm,source_frames=oracle(shapes,1,True,sequences=source)
        off=len(gold);gold.extend(pcm);frames=[]
        for i,(t,s) in enumerate(zip(target,source)):
            data=packet(i,t,0,shapes[i],1,3,True,source_sequence=s)
            row=dict(offset=len(blob),bytes=len(data),samples=SAMPLES[t]);blob.extend(data);frames.append(row)
            mono=packed('0000000'+channel(i,s,shapes[i],0,True,False)+'111')
            source_frames[i].update(offset=len(blob),bytes=len(mono));blob.extend(mono)
        asc=config(1,3)
        case=dict(name=name,slots=16,bands=32,asc=asc.hex(),channels=1,frames=frames,source_frames=source_frames,source_asc=packed(field(3,5)+frequency(24000)+'0001'+'000').hex(),pcm_offset=off,pcm_bytes=len(pcm),samples=6144,container_rate=24000,container_frame_samples=1472,durations=[SAMPLES[s] for s in target],target_sequences=target,source_sequences=source)
        case['video']=video_fixture([case],blob,channels=1,filename=f'aac-ssr-alignment-{name}-synthetic.mp4')
        source_case=dict(case,asc=case['source_asc'],frames=source_frames,durations=[SAMPLES[s] for s in source])
        case['source_video']=video_fixture([source_case],blob,channels=1,filename=f'aac-ssr-alignment-{name}-source-synthetic.mp4')
        cases.append(case)
    channels_cases=[]
    target=[0,1,2,3,0,0];source=[2,3,0,1,2,2]
    shapes=[i%2 for i in range(6)]
    left,_=oracle([0]*6,1,True,sequences=target)
    right,_=oracle(shapes,1,True,sequences=source)
    pcm=b''.join(a+b for a,b in zip([left[i:i+4] for i in range(0,len(left),4)],[right[i:i+4] for i in range(0,len(right),4)]))
    off=len(gold);gold.extend(pcm);frames=[]
    for i,(t,r) in enumerate(zip(target,source)):
        data=packed('0010000'+'0'+channel(i,t,0,0,True,False)+channel(i,r,shapes[i],0,True,False)+'111')
        frames.append(dict(offset=len(blob),bytes=len(data),samples=SAMPLES[t]));blob.extend(data)
    case=dict(name='independent-cpe-windows',slots=16,bands=32,asc=packed(field(3,5)+frequency(24000)+'0010'+'000').hex(),channels=2,frames=frames,pcm_offset=off,pcm_bytes=len(pcm),samples=6144,container_rate=24000,container_frame_samples=1472,durations=[SAMPLES[t] for t in target])
    case['video']=video_fixture([case],blob,channels=2,filename='aac-ssr-alignment-independent-cpe-synthetic.mp4');channels_cases.append(case)
    fixed_cases=[]
    for original in cases+channels_cases:
        if original['durations']==[1024]*6:continue
        fixed=dict(original,name=original['name']+'-fixed-clock',durations=[1024]*6)
        fixed['video']=video_fixture([fixed],blob,channels=fixed['channels'],filename='aac-ssr-alignment-'+fixed['name']+'-synthetic.mp4')
        fixed['matroska']=matroska_fixture(fixed,blob)
        fixed_cases.append(fixed)
    original=next(c for c in fixed_cases if c['name']=='late-source-behind-fixed-clock')
    tail=dict(original,name='late-source-behind-trimmed-tail',durations=[1024]*5+[600],samples=5720)
    tail.pop('matroska')
    tail['video']=video_fixture([tail],blob,filename='aac-ssr-alignment-trimmed-tail-synthetic.mp4')
    tail_cases=[tail]
    adts=bytearray()
    for row in channels_cases[0]['frames']:
        payload=blob[row['offset']:row['offset']+row['bytes']]
        header=field(0xfff,12)+'0'+'00'+'1'+field(2,2)+frequency(24000)+'0'+field(2,3)+'0000'+field(len(payload)+7,13)+field(0x7ff,11)+'00'
        assert len(header)==56
        adts.extend(packed(header)+payload)
    name='aac-ssr-alignment-independent-cpe-synthetic.aac';(DEST/name).write_bytes(adts)
    channels_cases[0]['adts']=dict(file=name,sha256=hashlib.sha256(adts).hexdigest())
    incomplete=dict(cases[0],frames=cases[0]['frames'][:2],durations=cases[0]['durations'][:2])
    incomplete['video']=video_fixture([incomplete],blob,filename='aac-ssr-alignment-incomplete-synthetic.mp4')
    incomplete['error']='SSR PCM alignment incomplete at stream end'
    roster=dict(cases[0],frames=[dict(row) for row in cases[0]['frames'][:3]],durations=cases[0]['durations'][:3])
    payload=packet(2,0,0,0,1,3,True,tags=())
    roster['frames'][2].update(offset=len(blob),bytes=len(payload));blob.extend(payload)
    roster['video']=video_fixture([roster],blob,filename='aac-ssr-alignment-roster-change-synthetic.mp4')
    roster['error']='AAC SSR aligned coupling roster changes require lane continuity'
    arrival_cases=[]
    for old,new,start,new_sequences,suffix in [
        (15,1,2,[0]*4,''),(1,15,2,[0]*4,''),
        (15,1,3,[1,2,3],'-start'),(1,15,3,[1,2,3],'-start'),
        (15,1,2,[2,3,1,2],'-short'),(1,15,2,[2,3,1,2],'-short'),
    ]:
        base=cases[0]
        earlier=gold[base['pcm_offset']:base['pcm_offset']+base['pcm_bytes']]
        later,_=oracle([i%2 for i in range(start,6)],1,True,lambda f,s,c:spectrum(f+start,s,c),sequences=new_sequences)
        later=bytes(start*1024*4)+later
        pcm=b''.join(struct.pack('<f',a+b) for (a,),(b,) in zip(struct.iter_unpack('<f',earlier),struct.iter_unpack('<f',later)))
        off=len(gold);gold.extend(pcm);frames=[]
        for i,seq in enumerate(base['source_sequences']):
            target='0000000'+silent(0,0,0,True,False)
            specs=[(old,seq,i%2)] + ([(new,new_sequences[i-start],i%2)] if i>=start else [])
            # Current packet element order changes; old/new histories are tags,
            # independent of order or their positions in the canonical queue.
            if i%2:specs.reverse()
            sources=''.join('010'+field(tag,4)+'1'+'000'+'0'+field(0,4)+'0'+'0'+'10'+channel(i,s,shape,0,True,False) for tag,s,shape in specs)
            data=packed((sources+target if i%2 else target+sources)+'111')
            frames.append(dict(offset=len(blob),bytes=len(data),samples=1024));blob.extend(data)
        arrival=dict(name=f'late-cce-{new}-after-{old}{suffix}',slots=16,bands=32,channels=1,asc=config(1,3,(1,15)).hex(),frames=frames,pcm_offset=off,pcm_bytes=len(pcm),samples=6144,container_rate=24000,container_frame_samples=1024,durations=[1024]*6,arrival_index=start,new_sequences=new_sequences,old_tag=old,new_tag=new)
        arrival['video']=video_fixture([arrival],blob,filename='aac-ssr-alignment-'+arrival['name']+'-synthetic.mp4')
        arrival['matroska']=matroska_fixture(arrival,blob)
        arrival_cases.append(arrival)
    (DEST/'aac-ssr-alignment-packets.bin').write_bytes(blob);(DEST/'aac-ssr-alignment-pcm.f32le').write_bytes(gold)
    (DEST/'aac-ssr-alignment.json').write_text(json.dumps(dict(cases=cases,channels_cases=channels_cases,fixed_cases=fixed_cases,tail_cases=tail_cases,incomplete=incomplete,roster=roster,arrival_cases=arrival_cases,packet_sha256=hashlib.sha256(blob).hexdigest(),pcm_sha256=hashlib.sha256(gold).hexdigest(),qualification='native coupled MP4 and timed playback acceptance including delayed EOF'),indent=2)+'\n')
    print(len(cases),'coupled SSR alignment scenarios; one independently switched CPE and one unfinished video')
if __name__=='__main__':main()
