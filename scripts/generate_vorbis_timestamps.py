#!/usr/bin/env python3
"""Authored silent Vorbis modes/granules and short HEVC video; offline/no encoders."""
import struct,json,hashlib
from generate_hevc_matroska_formats import D,unpack
from generate_ffv1_gray_fixtures import element,uint

def bits(fields):
    out=bytearray();at=0
    for value,width in fields:
        for i in range(width):
            if at%8==0:out.append(0)
            out[at//8]|=((value>>i)&1)<<(at%8);at+=1
    return bytes(out)
def page(payload,granule,flags,sequence,serial):
    laces=bytes([255]*(len(payload)//255)+[len(payload)%255])
    data=bytearray(b'OggS'+bytes([0,flags])+struct.pack('<qII',granule,serial,sequence)+bytes(4)+bytes([len(laces)])+laces+payload)
    crc=0
    for b in data:
        crc^=b<<24
        for _ in range(8):crc=((crc<<1)^0x04c11db7 if crc&0x80000000 else crc<<1)&0xffffffff
    data[22:26]=struct.pack('<I',crc);return bytes(data)
def main():
    ident=b'\x01vorbis'+struct.pack('<IBIiiiBB',0,2,8000,0,0,0,0x86,1)
    comment=b'\x03vorbis'+bytes(8)+b'\x01'
    fields=[(5,8)]+[(b,8)for b in b'vorbis']+[(0,8),(0x564342,24),(1,16),(1,24),(0,1),(0,1),(0,5),(0,4),(0,6),(0,16),(0,6),(1,16),(0,5),(0,2),(6,4),(0,6),(0,16),(0,24),(128,24),(31,24),(0,6),(0,8),(0,3),(0,1),(0,6),(0,16),(0,1),(0,1),(0,2),(0,8),(0,8),(0,8),(2,6)]
    for long in [0,0,1]:fields.extend([(long,1),(0,16),(0,16),(0,8)])
    setup=bits(fields+[(1,1)])
    modes=[0,2,1,2,2,0];packets=[]
    for i,mode in enumerate(modes):
        header=[(0,1),(mode,2)]
        if mode==2:header.extend([(int(i>0 and modes[i-1]==2),1),(int(i+1<len(modes)and modes[i+1]==2),1)])
        packets.append(bits(header+[(0,1),(0,1)]))
    def stream(offset,serial):
        data=page(ident,0,2,0,serial)+page(comment,0,0,1,serial)+page(setup,0,0,2,serial)
        ends=[0,80,160,240,368,448]
        for i,p in enumerate(packets):data+=page(p,(411+offset if i==5 else ends[i]+offset if i==1 else -1),4 if i==5 else 0,i+3,serial)
        return data
    names=[]
    for label,offset in [('zero',0),('initial-trim',-17),('positive-origin',23)]:
        name='vorbis-timestamps-'+label+'.ogg';data=stream(offset,0x46565430);(D/name).write_bytes(data);names.append(dict(file=name,offset=offset,end=411+offset,sha256=hashlib.sha256(data).hexdigest()))
    chained=stream(0,0x46565430)+stream(-17,0x46565431);(D/'vorbis-timestamps-chained.ogg').write_bytes(chained)
    changed_fields=fields[:-12]
    for long in [1,0,0]:changed_fields.extend([(long,1),(0,16),(0,16),(0,8)])
    changed_setup=bits(changed_fields+[(1,1)])
    changed_packets=[]
    for i,mode in enumerate(modes):
        header=[(0,1),({0:1,1:2,2:0}[mode],2)]
        if mode==2:header.extend([(int(i>0 and modes[i-1]==2),1),(int(i+1<len(modes)and modes[i+1]==2),1)])
        changed_packets.append(bits(header+[(0,1),(0,1)]))
    second=page(ident,0,2,0,0x46565432)+page(comment,0,0,1,0x46565432)+page(changed_setup,0,0,2,0x46565432)
    for i,p in enumerate(changed_packets):second+=page(p,394 if i==5 else 63 if i==1 else -1,4 if i==5 else 0,i+3,0x46565432)
    (D/'vorbis-timestamps-reconfigured.ogg').write_bytes(stream(0,0x46565430)+second)
    def parallel(skeleton=False,serial_base=0x46565440):
        serials=[serial_base,serial_base+1]
        data=page(b'fishead\0'+bytes(56),0,2,0,serial_base+2)if skeleton else b''
        data+=b''.join(page(ident,0,2,0,serial)for serial in serials)
        for sequence,payloads in [(1,[comment,comment]),(2,[setup,changed_setup])]:
            data+=b''.join(page(payload,0,0,sequence,serial)for payload,serial in zip(payloads,serials))
        for i,(a,b)in enumerate(zip(packets,changed_packets)):
            for track,payload in enumerate([a,b]):
                granule=([411,394][track]if i==5 else [80,63][track]if i==1 else -1)
                data+=page(payload,granule,4 if i==5 else 0,i+3,serials[track])
        if skeleton:data+=page(b'',0,4,1,serial_base+2)
        return data
    (D/'vorbis-timestamps-parallel.ogg').write_bytes(parallel())
    (D/'vorbis-timestamps-skeleton-parallel.ogg').write_bytes(parallel(True))
    (D/'vorbis-timestamps-parallel-chained.ogg').write_bytes(parallel()+parallel(serial_base=0x46565450))
    def source_pages(data):
        result=[];at=0
        while at<len(data):
            count=data[at+26];start=at+27+count;end=start+sum(data[at+27:start]);result.append((data[start:end],struct.unpack_from('<q',data,at+6)[0],data[at+5]));at=end
        return result
    opus_pages=source_pages((D/'opus-surround-presentation.opus').read_bytes())
    vorbis_pages=source_pages(stream(0,0x46565430))
    for label,streams in [('opus-first',[opus_pages,vorbis_pages]),('vorbis-first',[vorbis_pages,opus_pages])]:
        mixed=b''
        for track,parts in enumerate(streams):mixed+=page(parts[0][0],parts[0][1],parts[0][2],0,0x46565460+track)
        for sequence in range(1,max(map(len,streams))):
            for track,parts in enumerate(streams):
                if sequence<len(parts):payload,granule,flags=parts[sequence];mixed+=page(payload,granule,flags,sequence,0x46565460+track)
        (D/('ogg-mixed-'+label+'.ogg')).write_bytes(mixed)
    large_ident=ident[:28]+bytes([0x96])+ident[29:]
    large=page(large_ident,0,2,0,0x46565433)+page(comment,0,0,1,0x46565433)+page(changed_setup,0,0,2,0x46565433)
    for i,p in enumerate(changed_packets):large+=page(p,778 if i==5 else 127 if i==1 else -1,4 if i==5 else 0,i+3,0x46565433)
    (D/'vorbis-timestamps-changing-blocks.ogg').write_bytes(stream(0,0x46565430)+large)
    private,video,w,h,rate=unpack(D/'hevc-pcm-reference-rext8.mp4')
    vt=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+element(0xe0,uint(0xb0,w)+uint(0xba,h))
    lace=bytes([2,len(ident),len(comment)])+ident+comment+setup
    audio=uint(0xd7,2)+uint(0x83,2)+element(0x86,b'A_VORBIS')+element(0x63a2,lace)+element(0xe1,element(0xb5,struct.pack('>d',8000.))+uint(0x9f,2))
    clusters=b''.join(element(0x1f43b675,uint(0xe7,i*40)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(video))
    clusters+=b''.join(element(0x1f43b675,uint(0xe7,t//8)+element(0xa3,b'\x82\0\0\x80'+p))for t,p in zip([0,0,80,160,240,368],packets))
    movie=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1654ae6b,element(0xae,vt)+element(0xae,audio))+clusters)
    changed_lace=bytes([2,len(ident),len(comment)])+ident+comment+changed_setup
    changed_audio=uint(0xd7,2)+uint(0x83,2)+element(0x86,b'A_VORBIS')+element(0x63a2,changed_lace)+element(0xe1,element(0xb5,struct.pack('>d',8000.))+uint(0x9f,2))
    changed_clusters=b''.join(element(0x1f43b675,uint(0xe7,i*40)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(video))
    changed_clusters+=b''.join(element(0x1f43b675,uint(0xe7,t//8)+element(0xa3,b'\x82\0\0\x80'+p))for t,p in zip([0,0,80,160,240,368],changed_packets))
    changed_movie=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1654ae6b,element(0xae,vt)+element(0xae,changed_audio))+changed_clusters)
    (D/'hevc-vorbis-reconfigured-companion.mkv').write_bytes(changed_movie)
    large_lace=bytes([2,len(large_ident),len(comment)])+large_ident+comment+changed_setup
    large_audio=uint(0xd7,2)+uint(0x83,2)+element(0x86,b'A_VORBIS')+element(0x63a2,large_lace)+element(0xe1,element(0xb5,struct.pack('>d',8000.))+uint(0x9f,2))
    large_clusters=b''.join(element(0x1f43b675,uint(0xe7,i*40)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(video))
    large_clusters+=b''.join(element(0x1f43b675,uint(0xe7,t//8)+element(0xa3,b'\x82\0\0\x80'+p))for t,p in zip([0,0,144,288,432,688],changed_packets))
    large_movie=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1654ae6b,element(0xae,vt)+element(0xae,large_audio))+large_clusters)
    (D/'hevc-vorbis-changing-blocks-companion.mkv').write_bytes(large_movie)
    wide=[]
    for channels in [1,2,4,6,7,8,9,33,255]:
        wide_ident=ident[:11]+bytes([channels])+ident[12:]
        wide_packets=[]
        for i,mode in enumerate(modes):
            header=[(0,1),(mode,2)]
            if mode==2:header.extend([(int(i>0 and modes[i-1]==2),1),(int(i+1<len(modes)and modes[i+1]==2),1)])
            wide_packets.append(bits(header+[(0,1)]*channels))
        serial=0x46565500+channels
        data=page(wide_ident,0,2,0,serial)+page(comment,0,0,1,serial)+page(setup,0,0,2,serial)
        for i,p in enumerate(wide_packets):data+=page(p,411 if i==5 else 80 if i==1 else -1,4 if i==5 else 0,i+3,serial)
        name=f'vorbis-channels-{channels}.ogg';(D/name).write_bytes(data)
        config=bytes([2,len(wide_ident),len(comment)])+wide_ident+comment+setup
        track=uint(0xd7,2)+uint(0x83,2)+element(0x86,b'A_VORBIS')+element(0x63a2,config)+element(0xe1,element(0xb5,struct.pack('>d',8000.))+uint(0x9f,channels))
        parts=b''.join(element(0x1f43b675,uint(0xe7,i*40)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(video))
        parts+=b''.join(element(0x1f43b675,uint(0xe7,t//8)+element(0xa3,b'\x82\0\0\x80'+p))for t,p in zip([0,0,80,160,240,368],wide_packets))
        contents=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1654ae6b,element(0xae,vt)+element(0xae,track))+parts)
        movie_name=f'hevc-vorbis-channels-{channels}.mkv';(D/movie_name).write_bytes(contents)
        wide.append(dict(file=name,channels=channels,sha256=hashlib.sha256(data).hexdigest(),video=movie_name,video_sha256=hashlib.sha256(contents).hexdigest()))
    channel_chains=[]
    for first,second in [(1,2),(2,1),(4,6),(7,8),(9,33),(33,9),(2,255)]:
        data=(D/f'vorbis-channels-{first}.ogg').read_bytes()+(D/f'vorbis-channels-{second}.ogg').read_bytes()
        name=f'vorbis-channel-chain-{first}-{second}.ogg';(D/name).write_bytes(data)
        channel_chains.append(dict(file=name,channels=[first,second],sha256=hashlib.sha256(data).hexdigest(),videos=[f'hevc-vorbis-channels-{first}.mkv',f'hevc-vorbis-channels-{second}.mkv']))
    rate_chains=[]
    for first,second,offset in [(8000,12000,0),(12000,8000,0),(44100,48000,0),(48000,44100,0),(8000,12000,-17),(8000,12000,23)]:
        links=[];companions=[]
        for link,(rate,trim) in enumerate([(first,0),(second,offset)]):
            rate_ident=ident[:12]+struct.pack('<I',rate)+ident[16:]
            serial=0x46565600+link
            data=page(rate_ident,0,2,0,serial)+page(comment,0,0,1,serial)+page(setup,0,0,2,serial)
            for i,p in enumerate(packets):data+=page(p,411+trim if i==5 else 80+trim if i==1 else -1,4 if i==5 else 0,i+3,serial)
            links.append(data)
            config=bytes([2,len(rate_ident),len(comment)])+rate_ident+comment+setup
            track=uint(0xd7,2)+uint(0x83,2)+element(0x86,b'A_VORBIS')+element(0x63a2,config)+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,2))
            parts=b''.join(element(0x1f43b675,uint(0xe7,i*40)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(video))
            parts+=b''.join(element(0x1f43b675,uint(0xe7,max(0,(t+trim)*1000//rate))+element(0xa3,b'\x82'+min(0,(t+trim)*1000//rate).to_bytes(2,'big',signed=True)+b'\x80'+p))for t,p in zip([0,0,80,160,240,368],packets))
            contents=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1654ae6b,element(0xae,vt)+element(0xae,track))+parts)
            movie_name=f'hevc-vorbis-rate-{rate}-offset-{trim}.mkv';(D/movie_name).write_bytes(contents);companions.append(movie_name)
        data=b''.join(links);name=f'vorbis-rate-chain-{first}-{second}-offset-{offset}.ogg';(D/name).write_bytes(data)
        rate_chains.append(dict(file=name,rates=[first,second],offset=offset,sha256=hashlib.sha256(data).hexdigest(),videos=companions))
    triple=[]
    for index,rate in enumerate([12000,8000,8000]):
        rate_ident=ident[:12]+struct.pack('<I',rate)+ident[16:];serial=0x46565700+index
        data=page(rate_ident,0,2,0,serial)+page(comment,0,0,1,serial)+page(setup,0,0,2,serial)
        for i,p in enumerate(packets):data+=page(p,411 if i==5 else 80 if i==1 else -1,4 if i==5 else 0,i+3,serial)
        triple.append(data)
    (D/'vorbis-rate-chain-fractional-three.ogg').write_bytes(b''.join(triple))
    # Repage the existing public synthetic 440 Hz tone, retaining packet and
    # granule boundaries. Never read private media or invoke an encoder.
    data=(D.parent/'ogg'/'vorbis-stereo.ogg').read_bytes();at=0;tone=[];pending=b''
    while at<len(data):
        count=data[at+26];laces=data[at+27:at+27+count];payload=at+27+count;completed=[]
        for length in laces:
            pending+=data[payload:payload+length];payload+=length
            if length<255:completed.append(pending);pending=b''
        granule=struct.unpack_from('<q',data,at+6)[0]
        for i,payload_bytes in enumerate(completed):tone.append((payload_bytes,granule if i==len(completed)-1 else -1))
        at=payload
    assert not pending
    tone_links={};tone_videos=[]
    for index,rate in enumerate([44100,32000]):
        tone_ident=tone[0][0][:12]+struct.pack('<I',rate)+tone[0][0][16:];serial=0x46565800+index
        payloads=[(tone_ident,tone[0][1])]+tone[1:]
        data=b''.join(page(p,g,2 if i==0 else 4 if i==len(payloads)-1 else 0,i,serial)for i,(p,g)in enumerate(payloads))
        name=f'vorbis-rate-tone-{rate}.ogg';(D/name).write_bytes(data);tone_links[rate]=data
        def lace_length(n):return bytes([255]*(n//255)+[n%255])
        config=b'\x02'+lace_length(len(tone_ident))+lace_length(len(tone[1][0]))+tone_ident+tone[1][0]+tone[2][0]
        track=uint(0xd7,2)+uint(0x83,2)+element(0x86,b'A_VORBIS')+element(0x63a2,config)+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,2))
        parts=b''.join(element(0x1f43b675,uint(0xe7,i*40)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(video))
        for i,(p,g) in enumerate(tone[3:]):
            block=b'\x82\0\0\x80'+p
            trim_ns=(89088-88256)*1000000000//rate if i==len(tone[3:])-1 else 0
            payload=element(0xa0,element(0xa1,block)+element(0x75a2,trim_ns.to_bytes(8,'big',signed=True)))if trim_ns else element(0xa3,block)
            parts+=element(0x1f43b675,uint(0xe7,max(0,i-1)*1024000//rate)+payload)
        contents=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1654ae6b,element(0xae,vt)+element(0xae,track))+parts)
        name=f'hevc-vorbis-rate-tone-{rate}.mkv';(D/name).write_bytes(contents);tone_videos.append(name)
    for first,second in [(44100,32000),(32000,44100)]:
        (D/f'vorbis-rate-tone-chain-{first}-{second}.ogg').write_bytes(tone_links[first]+tone_links[second])
    rate_extra_names=['vorbis-rate-chain-fractional-three.ogg']+[f'vorbis-rate-tone-{r}.ogg'for r in [44100,32000]]+[f'vorbis-rate-tone-chain-{a}-{b}.ogg'for a,b in [(44100,32000),(32000,44100)]]+tone_videos
    rate_extra=[dict(file=name,sha256=hashlib.sha256((D/name).read_bytes()).hexdigest())for name in rate_extra_names]
    shape_links=[];shape_videos=[]
    for link,(channels,rate) in enumerate([(1,8000),(2,12000)]):
        original=source_pages((D/f'vorbis-channels-{channels}.ogg').read_bytes())
        local_ident=original[0][0][:12]+struct.pack('<I',rate)+original[0][0][16:]
        payloads=[(local_ident,original[0][1],original[0][2])]+original[1:]
        data=b''.join(page(p,g,flags,i,0x46565900+link)for i,(p,g,flags)in enumerate(payloads));shape_links.append(data)
        config=bytes([2,len(local_ident),len(comment)])+local_ident+comment+setup
        track=uint(0xd7,2)+uint(0x83,2)+element(0x86,b'A_VORBIS')+element(0x63a2,config)+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,channels))
        parts=b''.join(element(0x1f43b675,uint(0xe7,i*40)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(video))
        for i,((p,_,_),t) in enumerate(zip(payloads[3:],[0,0,80,160,240,368])):
            block=b'\x82\0\0\x80'+p;pad=37*1000000000//rate if i==5 else 0
            payload=element(0xa0,element(0xa1,block)+element(0x75a2,pad.to_bytes(8,'big',signed=True)))if pad else element(0xa3,block)
            parts+=element(0x1f43b675,uint(0xe7,t*1000//rate)+payload)
        contents=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1654ae6b,element(0xae,vt)+element(0xae,track))+parts)
        name=f'hevc-vorbis-rate-channel-{link}.mkv';(D/name).write_bytes(contents);shape_videos.append(name)
    (D/'vorbis-rate-channel-chain.ogg').write_bytes(b''.join(shape_links))
    rate_extra += [dict(file=name,sha256=hashlib.sha256((D/name).read_bytes()).hexdigest())for name in ['vorbis-rate-channel-chain.ogg']+shape_videos]
    codec_chains=[]
    opus=(D/'opus-surround-presentation.opus').read_bytes()
    vorbis=(D/'vorbis-timestamps-zero.ogg').read_bytes()
    for label,links in [('vorbis-opus',[vorbis,opus]),('opus-vorbis',[opus,vorbis])]:
        data=b''.join(links);name=f'ogg-codec-chain-{label}.ogg';(D/name).write_bytes(data)
        codec_chains.append(dict(file=name,sha256=hashlib.sha256(data).hexdigest(),videos=['hevc-vorbis-timestamps-companion.mkv','hevc-opus-surround-companion.mkv']))
    def rebase_serials(data,base):
        out=b'';at=0;serials={}
        while at<len(data):
            count=data[at+26];start=at+27+count;end=start+sum(data[at+27:start]);old=struct.unpack_from('<I',data,at+14)[0]
            if old not in serials:serials[old]=base+len(serials)
            out+=page(data[start:end],struct.unpack_from('<q',data,at+6)[0],data[at+5],struct.unpack_from('<I',data,at+18)[0],serials[old]);at=end
        return out
    for label,parts in [('vorbis-opus-vorbis',[vorbis,opus,vorbis]),('opus-vorbis-opus',[opus,vorbis,opus]),('discrete-opus',[(D/'vorbis-channels-9.ogg').read_bytes(),opus]),('multiplex-swap',[(D/'ogg-mixed-opus-first.ogg').read_bytes(),(D/'ogg-mixed-vorbis-first.ogg').read_bytes()]),('initial-trim-opus',[(D/'vorbis-timestamps-initial-trim.ogg').read_bytes(),opus]),('positive-origin-opus',[(D/'vorbis-timestamps-positive-origin.ogg').read_bytes(),opus]),('tone-vorbis-opus',[(D/'vorbis-rate-tone-44100.ogg').read_bytes(),opus]),('tone-opus-vorbis',[opus,(D/'vorbis-rate-tone-44100.ogg').read_bytes()])]:
        data=b''.join(rebase_serials(part,0x46565a00+index*16)for index,part in enumerate(parts))
        name=f'ogg-codec-chain-{label}.ogg';(D/name).write_bytes(data)
        codec_chains.append(dict(file=name,sha256=hashlib.sha256(data).hexdigest(),videos=[('hevc-vorbis-channels-9.mkv'if label=='discrete-opus'else 'hevc-vorbis-initial-trim-tail.mkv'if label=='initial-trim-opus'else 'hevc-vorbis-positive-origin-tail.mkv'if label=='positive-origin-opus'else 'hevc-vorbis-rate-tone-44100.mkv'if label.startswith('tone-')else 'hevc-vorbis-timestamps-companion.mkv'),'hevc-opus-surround-companion.mkv']))
    companion='hevc-vorbis-timestamps-companion.mkv';(D/companion).write_bytes(movie)
    webm_variants=[]
    for label,padding,delay in [('initial-trim-tail',4625000,2125000),('fractional-tail-padding',1,0),('tail-padding',4625000,0),('head-padding',-2125000,0),('codec-delay',0,2125000)]:
        track=audio+(uint(0x56aa,delay)if delay else b'')
        parts=b''.join(element(0x1f43b675,uint(0xe7,i*40)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(video))
        for i,(t,p)in enumerate(zip([0,0,80,160,240,368],packets)):
            pad=padding if (padding>0 and i==5)or(padding<0 and i==1)else 0
            block=b'\x82\0\0\x80'+p
            payload=element(0xa0,element(0xa1,block)+element(0x75a2,pad.to_bytes(8,'big',signed=True)))if pad else element(0xa3,block)
            parts+=element(0x1f43b675,uint(0xe7,t//8)+payload)
        contents=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1654ae6b,element(0xae,vt)+element(0xae,track))+parts)
        name='hevc-vorbis-'+label+'.mkv';(D/name).write_bytes(contents);webm_variants.append(dict(file=name,padding_ns=padding,codec_delay_ns=delay,sha256=hashlib.sha256(contents).hexdigest()))
    parts=b''.join(element(0x1f43b675,uint(0xe7,i*320)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(video))
    for i,(t,p) in enumerate(zip([0,0,80,160,240,368],packets)):
        block=b'\x82\0\0\x80'+p
        payload=element(0xa0,element(0xa1,block)+element(0x75a2,(4625000).to_bytes(8,'big',signed=True)))if i==5 else element(0xa3,block)
        parts+=element(0x1f43b675,uint(0xe7,t+23)+payload)
    positive_video=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,125000))+element(0x1654ae6b,element(0xae,vt)+element(0xae,audio))+parts)
    (D/'hevc-vorbis-positive-origin-tail.mkv').write_bytes(positive_video)
    (D/'hevc-vorbis-endpoint-rewind.mkv').write_bytes((D/'hevc-vorbis-tail-padding.mkv').read_bytes())
    (D/'vorbis-timestamps.json').write_text(json.dumps(dict(generator=__file__,video=companion,modes=modes,durations=[0,80,80,80,128,80],fixtures=names,webm_variants=webm_variants,channel_variants=wide,channel_chains=channel_chains,codec_chains=codec_chains,rate_chains=rate_chains,rate_tone_videos=tone_videos,rate_extra=rate_extra,endpoint_rewind=dict(file="hevc-vorbis-endpoint-rewind.mkv",sha256=hashlib.sha256((D/"hevc-vorbis-endpoint-rewind.mkv").read_bytes()).hexdigest()),mixed=[dict(file="ogg-mixed-"+label+".ogg",opus_video="hevc-opus-surround-companion.mkv",vorbis_video="hevc-vorbis-timestamps-companion.mkv",sha256=hashlib.sha256((D/("ogg-mixed-"+label+".ogg")).read_bytes()).hexdigest())for label in ["opus-first","vorbis-first"]],parallel="vorbis-timestamps-parallel.ogg",parallel_chained="vorbis-timestamps-parallel-chained.ogg",skeleton_parallel="vorbis-timestamps-skeleton-parallel.ogg",chained_end=805,reconfigured="vorbis-timestamps-reconfigured.ogg",reconfigured_video="hevc-vorbis-reconfigured-companion.mkv",changing_blocks="vorbis-timestamps-changing-blocks.ogg",changing_blocks_end=1189,changing_blocks_video="hevc-vorbis-changing-blocks-companion.mkv"),indent=2)+'\n')
if __name__=='__main__':main()
