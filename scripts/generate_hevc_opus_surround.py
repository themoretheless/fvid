#!/usr/bin/env python3
"""Short authored HEVC + existing synthetic family-1 Opus, no codecs run."""
import json,hashlib,struct
from generate_hevc_matroska_timeline import fields,child as ebml_child,vint
from generate_hevc_matroska_formats import D,unpack
from generate_ffv1_gray_fixtures import element,uint

def main():
    segment=ebml_child((D/'opus-surround.webm').read_bytes(),0x18538067)
    audio=next(body for _,body in fields(ebml_child(segment,0x1654ae6b)) if dict(fields(body)).get(0x86)==b'A_OPUS')
    meta=dict(fields(audio));head=meta[0x63a2];assert head[9]==6 and head[18]==1
    number=int.from_bytes(meta[0xd7],'big');packets=[]
    for tag,cluster in fields(segment):
        if tag!=0x1f43b675:continue
        for k,v in fields(cluster):
            block=v if k==0xa3 else ebml_child(v,0xa1) if k==0xa0 else None
            if block is None:continue
            track,at=vint(block,0,True)
            if track!=number:continue
            assert block[at+2]&6==0
            packet=block[at+3:]
            assert packet[0]&3==0 and packet[0]>>3>=16 and (packet[0]>>3)%4==3
            packets.append(packet)
    packets=packets[:6];assert len(packets)==6
    delay=int.from_bytes(head[10:12],'little')*1000000000//48000
    audio=uint(0xd7,2)+uint(0x83,2)+element(0x86,b'A_OPUS')+element(0x63a2,head)+uint(0x56aa,delay)+uint(0x23e383,20000000)+element(0xe1,element(0xb5,struct.pack('>d',48000.))+uint(0x9f,6))
    private,video,w,h,rate=unpack(D/'hevc-pcm-reference-rext8.mp4');assert len(video)==3 and rate==25
    vt=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+uint(0x23e383,40000000)+element(0xe0,uint(0xb0,w)+uint(0xba,h))
    clusters=b''.join(element(0x1f43b675,uint(0xe7,i*40000000)+element(0xa3,b'\x81\0\0'+(b'\x80' if i==0 else b'\0')+p)) for i,p in enumerate(video))
    clusters+=b''.join(element(0x1f43b675,uint(0xe7,i*20000000)+element(0xa3,b'\x82\0\0\x80'+p)) for i,p in enumerate(packets))
    data=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,audio))+clusters)
    file='hevc-opus-surround-companion.mkv';(D/file).write_bytes(data)
    plain=b''.join(element(k,v) for k,v in fields(audio) if k!=0x23e383)
    untimed=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,plain))+clusters)
    (D/'hevc-opus-surround-no-duration.mkv').write_bytes(untimed)
    first=packets[0];assert first[0]&3==0
    length=first[1];prefix=2
    if length>=252:length+=4*first[2];prefix=3
    assert prefix+length<len(first)
    # Code 3, one CBR frame, seven pad octets, then the existing
    # self-delimited frame length and unchanged compressed frame.
    padded=bytes([(first[0]&252)|3,65,7])+first[1:prefix]+first[prefix:prefix+length]+bytes(7)+first[prefix+length:]
    padded_audio=b''.join(element(0x1f43b675,uint(0xe7,i*20000000)+element(0xa3,b'\x82\0\0\x80'+p)) for i,p in enumerate([padded]+packets[1:]))
    padded_video=b''.join(element(0x1f43b675,uint(0xe7,i*40000000)+element(0xa3,b'\x81\0\0'+(b'\x80' if i==0 else b'\0')+p)) for i,p in enumerate(video))
    with_padding=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,audio))+padded_video+padded_audio)
    (D/'hevc-opus-surround-padded-stream.mkv').write_bytes(with_padding)

    # Explicit discrete map: reorder, duplicate and silence output channels.
    discrete_head=head[:18]+bytes([255,head[19],head[20],5,0,0,255,2,1])
    discrete_audio=b''.join(element(k,discrete_head if k==0x63a2 else v) for k,v in fields(audio))
    discrete=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,discrete_audio))+clusters)
    (D/'hevc-opus-discrete-companion.mkv').write_bytes(discrete)
    remapped_head=discrete_head[:18]+bytes([1])+discrete_head[19:]
    remapped_audio=b''.join(element(k,remapped_head if k==0x63a2 else v) for k,v in fields(audio))
    remapped=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,remapped_audio))+clusters)
    (D/'hevc-opus-remapped-surround.mkv').write_bytes(remapped)
    (D/'opus-surround-native-packets.bin').write_bytes(b''.join(struct.pack('<I',len(p))+p for p in packets))

    wide_names=[]
    for channels in (9,255):
        mapping=bytes([5,0,0,255,2,1][i%6] for i in range(channels))
        wide_head=head[:9]+bytes([channels])+head[10:18]+bytes([255,head[19],head[20]])+mapping
        wide_audio=b''.join(element(k,wide_head if k==0x63a2 else element(0xb5,struct.pack('>d',48000.))+uint(0x9f,channels) if k==0xe1 else v) for k,v in fields(audio))
        wide=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,wide_audio))+clusters)
        name=f'hevc-opus-discrete-{channels}.mkv';(D/name).write_bytes(wide);wide_names.append(name)

    invalid_head=head[:18]+bytes([255,254,2])+head[21:]
    invalid_audio=b''.join(element(k,invalid_head if k==0x63a2 else v) for k,v in fields(audio))
    invalid=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,invalid_audio))+clusters)
    (D/'hevc-opus-invalid-decoded-count.mkv').write_bytes(invalid)

    import math
    mono=[struct.pack('<f',0.2*math.sin(2*math.pi*440*i/48000)) for i in range(3*960)]
    raw=b''.join(mono);(D/'opus-discrete-encoder-source.f32').write_bytes(raw)
    pcm_track=uint(0xd7,2)+uint(0x83,2)+element(0x86,b'A_PCM/FLOAT/IEEE')+uint(0x23e383,20000000)+element(0xe1,element(0xb5,struct.pack('>d',48000.))+uint(0x9f,1)+uint(0x6264,32))
    pcm_clusters=b''.join(element(0x1f43b675,uint(0xe7,i*20000000)+element(0xa3,b'\x82\0\0\x80'+raw[i*3840:(i+1)*3840])) for i in range(3))
    source_movie=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,pcm_track))+padded_video+pcm_clusters)
    (D/'hevc-opus-discrete-encoder-source.mkv').write_bytes(source_movie)
    (D/'opus-discrete-encoder-source.json').write_text(json.dumps(dict(video='hevc-opus-discrete-encoder-source.mkv',audio='opus-discrete-encoder-source.f32',sample_rate=48000,channels=1,sample_frames=2880,audio_packets=3,video_frames=3,video_sha256=hashlib.sha256(source_movie).hexdigest(),audio_sha256=hashlib.sha256(raw).hexdigest(),provenance='Authored 440 Hz sine float32 and owned HEVC PCM frames; no private media or external codecs.'),indent=2)+'\n')

    malformed=bytearray(packets[0]);at=0
    for _ in range(3):
        assert malformed[at]&3==0
        length=malformed[at+1];prefix=2
        if length>=252:length+=4*malformed[at+2];prefix=3
        at+=prefix+length
    malformed[at]^=8
    invalid_packets=[bytes(malformed)]+packets[1:]
    invalid_clusters=b''.join(element(0x1f43b675,uint(0xe7,i*20000000)+element(0xa3,b'\x82\0\0\x80'+p)) for i,p in enumerate(invalid_packets))
    invalid_timing=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,audio))+padded_video+invalid_clusters)
    (D/'hevc-opus-invalid-stream-duration.mkv').write_bytes(invalid_timing)

    def ogg_page(payload,granule,flags,sequence,serial=0x46564944):
        laces=bytes([255]*(len(payload)//255)+[len(payload)%255])
        page=bytearray(b'OggS'+bytes([0,flags])+struct.pack('<qII',granule,serial,sequence)+bytes(4)+bytes([len(laces)])+laces+payload)
        crc=0
        for byte in page:
            crc^=byte<<24
            for _ in range(8):crc=((crc<<1)^0x04c11db7 if crc&0x80000000 else crc<<1)&0xffffffff
        page[22:26]=struct.pack('<I',crc);return bytes(page)
    ogg_variants=[]
    for name,configuration,payloads in [('opus-surround-presentation.opus',head,packets),('opus-surround-padded-presentation.opus',head,[padded]+packets[1:])]:
        contents=ogg_page(configuration,0,2,0)+ogg_page(b'OpusTags'+bytes(8),0,0,1)
        contents+=b''.join(ogg_page(payload,(i+1)*960-(137 if i==5 else 0),4 if i==5 else 0,i+2) for i,payload in enumerate(payloads))
        (D/name).write_bytes(contents);ogg_variants.append(dict(file=name,channels=6,pre_skip=312,final_trim=137,sha256=hashlib.sha256(contents).hexdigest()))
    second_head=head[:10]+struct.pack('<H',648)+head[12:]
    second=ogg_page(second_head,0,2,0,0x46564945)+ogg_page(b'OpusTags'+bytes(8),0,0,1,0x46564945)
    second+=b''.join(ogg_page(payload,(i+1)*960-(211 if i==5 else 0),4 if i==5 else 0,i+2,0x46564945) for i,payload in enumerate([padded]+packets[1:]))
    chained=(D/'opus-surround-presentation.opus').read_bytes()+second
    (D/'opus-surround-chained.opus').write_bytes(chained)
    ogg_variants.append(dict(file='opus-surround-chained.opus',chains=2,pre_skips=[312,648],final_trims=[137,211],sha256=hashlib.sha256(chained).hexdigest()))
    parallel=b''.join(ogg_page(configuration,0,2,0,serial) for configuration,serial in [(head,0x46564944),(second_head,0x46564945)])
    def named_tags(title,language):
        comments=[('TITLE='+title).encode(),('LANGUAGE='+language).encode()]
        return b'OpusTags'+bytes(4)+struct.pack('<I',len(comments))+b''.join(struct.pack('<I',len(c))+c for c in comments)
    tagged=ogg_page(discrete_head,0,2,0)+ogg_page(named_tags('Discrete program','und'),0,0,1)
    tagged+=b''.join(ogg_page(payload,(i+1)*960-(137 if i==5 else 0),4 if i==5 else 0,i+2) for i,payload in enumerate(packets))
    (D/'opus-discrete-tagged.opus').write_bytes(tagged)
    ogg_variants.append(dict(file='opus-discrete-tagged.opus',video='hevc-opus-discrete-companion.mkv',channels=6,mapping_family=255,pre_skip=312,final_trim=137,sha256=hashlib.sha256(tagged).hexdigest()))
    parallel+=b''.join(ogg_page(named_tags(title,language),0,0,1,serial) for serial,title,language in [(0x46564944,'Left program','eng'),(0x46564945,'Right program','rus')])
    for i,payload in enumerate(packets):
        for serial in [0x46564944,0x46564945]:
            parallel+=ogg_page(payload,(i+1)*960-(137 if i==5 else 0),4 if i==5 else 0,i+2,serial)
    (D/'opus-surround-parallel.opus').write_bytes(parallel)
    ogg_variants.append(dict(file='opus-surround-parallel.opus',streams=2,pre_skips=[312,648],final_trims=[137,137],sha256=hashlib.sha256(parallel).hexdigest()))
    following=b''.join(ogg_page(configuration,0,2,0,serial) for configuration,serial in [(second_head,0x46564946),(head,0x46564947)])
    following+=b''.join(ogg_page(named_tags(title,language),0,0,1,serial) for serial,title,language in [(0x46564946,'Left continued','eng'),(0x46564947,'Right continued','rus')])
    for i,payload in enumerate([padded]+packets[1:]):
        for serial,trim in [(0x46564946,211),(0x46564947,137)]:
            following+=ogg_page(payload,(i+1)*960-(trim if i==5 else 0),4 if i==5 else 0,i+2,serial)
    (D/'opus-surround-parallel-chained.opus').write_bytes(parallel+following)
    ogg_variants.append(dict(file='opus-surround-parallel-chained.opus',links=2,tracks=2,sha256=hashlib.sha256(parallel+following).hexdigest()))
    reconfigured=bytearray(remapped_head);reconfigured[10:12]=struct.pack('<H',648);reconfigured[16:18]=struct.pack('<h',1536);reconfigured=bytes(reconfigured)
    reconfigured_audio=b''.join(element(k,reconfigured if k==0x63a2 else (648*1000000000//48000).to_bytes(4,'big') if k==0x56aa else v) for k,v in fields(audio))
    reconfigured_movie=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,reconfigured_audio))+padded_video+padded_audio)
    (D/'hevc-opus-reconfigured-companion.mkv').write_bytes(reconfigured_movie)
    changed=ogg_page(reconfigured,0,2,0,0x46564945)+ogg_page(b'OpusTags'+bytes(8),0,0,1,0x46564945)
    changed+=b''.join(ogg_page(payload,(i+1)*960-(211 if i==5 else 0),4 if i==5 else 0,i+2,0x46564945) for i,payload in enumerate([padded]+packets[1:]))
    reconfigured_chain=(D/'opus-surround-presentation.opus').read_bytes()+changed
    (D/'opus-surround-reconfigured.opus').write_bytes(reconfigured_chain)
    ogg_variants.append(dict(file='opus-surround-reconfigured.opus',chains=2,second_gain_q8=1536,second_pre_skip=648,second_mapping=list(reconfigured[21:]),second_video='hevc-opus-reconfigured-companion.mkv',second_video_sha256=hashlib.sha256(reconfigured_movie).hexdigest(),sha256=hashlib.sha256(reconfigured_chain).hexdigest()))
    (D/'opus-ogg-presentation.json').write_text(json.dumps(dict(variants=ogg_variants,provenance='Existing authored six-channel Opus packets also present in short HEVC/Opus synthetic video; pure Python Ogg framing and CRC, no codec runs.'),indent=2)+'\n')

    quad_head=head[:9]+bytes([4])+head[10:18]+bytes([1,2,2,0,1,2,3])
    quad_packets=[]
    for packet in packets:
        at=0;parts=[]
        for stream in range(2):
            assert packet[at]&3==0
            size=packet[at+1];prefix=2
            if size>=252:size+=4*packet[at+2];prefix=3
            parts.append(packet[at:at+prefix+size] if stream==0 else packet[at:at+1]+packet[at+prefix:at+prefix+size])
            at+=prefix+size
        quad_packets.append(b''.join(parts))
    quad_audio=b''.join(element(k,quad_head if k==0x63a2 else element(0xb5,struct.pack('>d',48000.))+uint(0x9f,4) if k==0xe1 else v) for k,v in fields(audio))
    quad_clusters=b''.join(element(0x1f43b675,uint(0xe7,i*20000000)+element(0xa3,b'\x82\0\0\x80'+p)) for i,p in enumerate(quad_packets))
    quad_movie=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,quad_audio))+padded_video+quad_clusters)
    (D/'hevc-opus-quad-companion.mkv').write_bytes(quad_movie)
    pcm_source=(D/'opus-discrete-encoder-source.f32').read_bytes()
    precision_fixtures=[]
    for bits,code in [(32,'f'),(64,'d')]:
        pcm=b''.join(struct.pack('<'+code,value*scale) for (value,) in struct.iter_unpack('<f',pcm_source) for scale in [1.,0.5,0.25,-0.5])
        block=4*(bits//8)
        fmt=struct.pack('<HHIIHHHHI',0xfffe,4,48000,48000*block,block,bits,22,bits,0x33)+bytes.fromhex('0300000000001000800000aa00389b71')
        wave=b'RIFF'+struct.pack('<I',4+8+len(fmt)+8+len(pcm))+b'WAVEfmt '+struct.pack('<I',len(fmt))+fmt+b'data'+struct.pack('<I',len(pcm))+pcm
        name=f'quad-rematrix-float{bits}.wav';(D/name).write_bytes(wave)
        precision_fixtures.append(dict(file=name,bits=bits,channels=4,sample_frames=2880,mask=51,sha256=hashlib.sha256(wave).hexdigest()))

    def opus_segment(configuration,payloads,serial,trim):
        contents=ogg_page(configuration,0,2,0,serial)+ogg_page(b'OpusTags'+bytes(8),0,0,1,serial)
        return contents+b''.join(ogg_page(payload,(i+1)*960-(trim if i==5 else 0),4 if i==5 else 0,i+2,serial) for i,payload in enumerate(payloads))
    quad_second=quad_head[:10]+struct.pack('<H',648)+quad_head[12:]
    six_second=head[:10]+struct.pack('<H',648)+head[12:]
    (D/'opus-surround-to-quad.opus').write_bytes(opus_segment(head,packets,0x46564944,137)+opus_segment(quad_second,quad_packets,0x46564945,211))
    (D/'opus-quad-to-surround.opus').write_bytes(opus_segment(quad_head,quad_packets,0x46564944,137)+opus_segment(six_second,packets,0x46564945,211))

    mono_head=head[:9]+bytes([1])+head[10:18]+bytes([0])
    mono_packets=[]
    for packet in packets:
        at=0
        for _ in range(3):
            size=packet[at+1];prefix=2
            if size>=252:size+=4*packet[at+2];prefix=3
            at+=prefix+size
        mono_packets.append(packet[at:])
    mono_audio=b''.join(element(k,mono_head if k==0x63a2 else element(0xb5,struct.pack('>d',48000.))+uint(0x9f,1) if k==0xe1 else v) for k,v in fields(audio))
    mono_clusters=b''.join(element(0x1f43b675,uint(0xe7,i*20000000)+element(0xa3,b'\x82\0\0\x80'+p)) for i,p in enumerate(mono_packets))
    mono_movie=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,vt)+element(0xae,mono_audio))+padded_video+mono_clusters)
    (D/'hevc-opus-mono-companion.mkv').write_bytes(mono_movie)
    mono_second=mono_head[:10]+struct.pack('<H',648)+mono_head[12:]
    (D/'opus-mono-to-quad.opus').write_bytes(opus_segment(mono_head,mono_packets,0x46564944,137)+opus_segment(quad_second,quad_packets,0x46564945,211))
    (D/'opus-quad-to-mono.opus').write_bytes(opus_segment(quad_head,quad_packets,0x46564944,137)+opus_segment(mono_second,mono_packets,0x46564945,211))
    layout_cases=[]
    for name,mask in [('opus-surround-to-quad.opus',63),('opus-quad-to-surround.opus',63),('opus-mono-to-quad.opus',55),('opus-quad-to-mono.opus',55)]:
        layout_cases.append(dict(file=name,output_mask=mask,output_channels=bin(mask).count('1'),sha256=hashlib.sha256((D/name).read_bytes()).hexdigest()))
    (D/'opus-changing-layouts.json').write_text(json.dumps(dict(cases=layout_cases,pre_skips=[312,648],tail_trims=[137,211],provenance='Authored HEVC/Opus video companions; selected coupled and mono elementary streams from existing synthetic packets. No private data, external codec execution or network.'),indent=2)+'\n')

    (D/'opus-quad-speakers.json').write_text(json.dumps(dict(video='hevc-opus-quad-companion.mkv',channels=4,streams=2,coupled_streams=2,expected_wave_mask=51,precision_fixtures=precision_fixtures,source_six_channel_wave_indices=[0,1,4,5],sha256=hashlib.sha256(quad_movie).hexdigest(),provenance='First two authored coupled Opus streams, removing the final self-delimited length. Owned HEVC PCM video, no private material or external codecs.'),indent=2)+'\n')

    variants=[dict(file=n,channels=255 if '-255.' in n else 9 if '-9.' in n else 6,mapping_family=255 if 'discrete' in n or 'invalid-decoded' in n else 1,sha256=hashlib.sha256((D/n).read_bytes()).hexdigest()) for n in [file,'hevc-opus-surround-no-duration.mkv','hevc-opus-surround-padded-stream.mkv','hevc-opus-discrete-companion.mkv','hevc-opus-remapped-surround.mkv','hevc-opus-invalid-decoded-count.mkv','hevc-opus-invalid-stream-duration.mkv']+wide_names]
    for variant in variants:
        if variant['file']=='hevc-opus-invalid-stream-duration.mkv':
            variant.update(expected_error='multistream packet durations disagree',changed_stream=3)
        if variant['file']=='hevc-opus-invalid-decoded-count.mkv':
            variant.update(stream_count=254,coupled_count=2,expected_error='OpusHead declares more than 255 decoded channels')

    (D/'hevc-opus-surround-companion.json').write_text(json.dumps(dict(file=file,variants=variants,channels=6,mapping_family=1,video_frames=3,audio_packets=6,sha256=hashlib.sha256(data).hexdigest(),provenance='Owned PCM HEVC and six saved synthetic Opus family-1 packets, normalized 20 ms coded clock and explicit durations to isolate transport-header refusal. No private data, codecs run, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
