"""Owned VP9/Matroska fixtures from separately frozen synthetic audio vectors.
No encoding program, subprocess, network or generated output fixture is read.
External audio reference refresh is a separate explicit benchmark helper.
"""
import hashlib,json,struct
from pathlib import Path
from generate_ffv1_gray_fixtures import element,uint
from generate_vp9_chroma import frame
from matroska_sample_clock_reference import load
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/matroska-audio-sample-clocks'

def signed(tag,value):
    return element(tag,int(value).to_bytes(8,'big',signed=True))

def movie(case,private,packets):
    audio=element(0xB5,struct.pack('>d',float(case['sample_rate'])))+uint(0x9F,case['channels'])
    if case['bit_depth'] is not None:audio+=uint(0x6264,case['bit_depth'])
    track=uint(0xD7,2)+uint(0x73C5,2)+uint(0x83,2)+element(0x86,case['codec'].encode())+element(0xE1,audio)
    if private:track+=element(0x63A2,private)
    for key,tag in [('codec_delay_ns',0x56AA),('seek_preroll_ns',0x56BB),('default_duration_ns',0x23E383)]:
        if case[key]:track+=uint(tag,case[key])
    video=uint(0xD7,1)+uint(0x73C5,1)+uint(0x83,1)+element(0x86,b'V_VP9')
    video+=uint(0x23E383,20_000_000)+element(0xE0,uint(0xB0,16)+uint(0xBA,16))
    tick=case['timestamp_scale_ns'];assert tick==1_000_000
    picture=frame(8,'420',16,16)
    events=[(i*20,1,i,element(0xA3,b'\x81\0\0\x80'+picture))for i in range(12)]
    for i,(description,packet)in enumerate(zip(case['packets'],packets)):
        point=description['timestamp_ticks'];relative=min(0,point)
        block=b'\x82'+struct.pack('>hB',relative,description['flags'])+packet
        if description['group']:
            body=element(0xA1,block)
            if description['duration_ticks'] is not None:body+=uint(0x9B,description['duration_ticks'])
            if description['discard_padding_ns']:body+=signed(0x75A2,description['discard_padding_ns'])
            body+=b''.join(signed(0xFB,v)for v in description['references_ticks'])
            block=element(0xA0,body)
        else:
            assert not description['discard_padding_ns'] and description['duration_ticks'] is None
            block=element(0xA3,block)
        events.append((point,2,i,block))
    clusters=b''.join(element(0x1F43B675,uint(0xE7,max(0,point))+block)for point,_,_,block in sorted(events))
    header=element(0x1A45DFA3,uint(0x4286,1)+uint(0x42F7,1)+uint(0x42F2,4)+uint(0x42F3,8)+element(0x4282,b'matroska')+uint(0x4287,4)+uint(0x4285,2))
    info=uint(0x2AD7B1,tick)+element(0x4489,struct.pack('>d',case['duration_ticks']))
    return header+element(0x18538067,element(0x1549A966,info)+element(0x1654AE6B,element(0xAE,video)+element(0xAE,track))+clusters)

def generate():
    files={};cases=[]
    for case,private,packets in load():
        name=case['name']+'.mkv';data=movie(case,private,packets);files[name]=data
        cases.append(dict(file=name,codec=case['codec'],reference_encoder=case['reference_encoder'],reference_payload_sha256=case['payload_sha256'],reference_movie_sha256=case['source_movie_sha256'],sha256=hashlib.sha256(data).hexdigest()))
    files['manifest.json']=(json.dumps(dict(provenance='Owned neutral VP9 and Matroska construction from separately frozen original synthetic nonzero audio packets. All six audio payloads/configuration/timing are preserved. No encoder, FFmpeg, network or generated output is read. Reference encoding refresh is a separate explicit benchmark, not ordinary fixture generation.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
