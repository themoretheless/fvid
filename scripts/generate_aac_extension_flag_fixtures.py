#!/usr/bin/env python3
"""Own AAC ASC extensionFlag videos; no private media, network or codec tool."""
import hashlib,json
from generate_aac_ssr_fixtures import DEST,field,frequency,packed,packet,SEQUENCES
from generate_aac_ssr_coupling_fixtures import program,packet as coupling_packet
from generate_he_aac_packet_fixtures import video_fixture

def asc(object_type,channels,extension,flag3=0,pce=False,sync=False):
    bits=field(object_type,5)+frequency(24000)+field(0 if pce else channels,4)+'00'+field(extension,1)
    if pce:bits=program(bits,channels,3)
    if extension:bits+=field(flag3,1)
    if sync:bits+=field(0x2b7,11)+field(5,5)+'0'
    return packed(bits)

def main():
    blob=bytearray();cases=[];invalid=[]
    for name,obj,channels,pce,sync in [
        ('lc-indexed',2,1,False,False),('lc-sync-absent',2,1,False,True),
        ('ssr-indexed',3,2,False,False),('ssr-pce',3,1,True,False),
    ]:
        frames=[]
        for i,seq in enumerate(SEQUENCES):
            payload=coupling_packet(i,seq,i%2,i%2,1,3,True) if pce else packet(i,seq,i%2,channels,obj==3)
            frames.append(dict(offset=len(blob),bytes=len(payload),samples=1024));blob.extend(payload)
        case=dict(name=name,slots=16,bands=32,channels=channels,object_type=obj,frames=frames,asc=asc(obj,channels,True,pce=pce,sync=sync).hex(),baseline_asc=asc(obj,channels,False,pce=pce,sync=sync).hex(),samples=6144,pcm_offset=0,container_rate=24000,container_frame_samples=1024,durations=[1024]*6)
        case['video']=video_fixture([case],blob,channels=channels,filename='aac-extension-flag-'+name+'-synthetic.mp4')
        baseline=dict(case,asc=case['baseline_asc']);case['baseline_video']=video_fixture([baseline],blob,channels=channels,filename='aac-extension-flag-'+name+'-baseline-synthetic.mp4')
        cases.append(case)
        bad=dict(case,asc=asc(obj,channels,True,1,pce=pce,sync=sync).hex(),error='AAC extensionFlag3 must be zero')
        bad['video']=video_fixture([bad],blob,channels=channels,filename='aac-extension-flag-'+name+'-invalid-synthetic.mp4');invalid.append(bad)
    references=json.loads((DEST/'he-aac-sbr-packets.json').read_text())['cases']
    reference=next(c for c in references if c['slots']==16 and c['bands']==64 and c['signalling']=='explicit')
    ps=json.loads((DEST/'aac-ps-playback-oracles.json').read_text())['cases'][0]
    for name,explicit,is_ps in [('he-explicit',True,False),('he-sync',False,False),('ps-explicit',True,True),('ps-sync',False,True)]:
        source=ps if is_ps else reference
        source_blob=(DEST/(ps['packet_file'] if is_ps else 'he-aac-sbr-packets.bin')).read_bytes()
        frames=[]
        for row in source['frames']:
            payload=source_blob[row['offset']:row['offset']+row['bytes']]
            frames.append(dict(offset=len(blob),bytes=len(payload),samples=2048));blob.extend(payload)
        def he_config(extension,flag3=0):
            if explicit:
                bits=field(29 if is_ps else 5,5)+frequency(24000)+field(1,4)+frequency(48000)+field(2,5)+'00'+field(extension,1)
            else:bits=field(2,5)+frequency(24000)+field(1,4)+'00'+field(extension,1)
            if extension:bits+=field(flag3,1)
            if not explicit:bits+=field(0x2b7,11)+field(5,5)+'1'+frequency(48000)+(field(0x548,11)+'1' if is_ps else '')
            return packed(bits)
        case=dict(name=name,slots=16,bands=64,channels=2 if is_ps else 1,object_type=2,ps=is_ps,frames=frames,asc=he_config(True).hex(),baseline_asc=he_config(False).hex(),samples=6144,pcm_offset=0,container_rate=48000,container_frame_samples=2048,durations=[2048]*3)
        case['video']=video_fixture([case],blob,channels=case['channels'],filename='aac-extension-flag-'+name+'-synthetic.mp4')
        case['baseline_video']=video_fixture([dict(case,asc=case['baseline_asc'])],blob,channels=case['channels'],filename='aac-extension-flag-'+name+'-baseline-synthetic.mp4');cases.append(case)
        bad=dict(case,asc=he_config(True,1).hex(),error='AAC extensionFlag3 must be zero')
        bad['video']=video_fixture([bad],blob,channels=case['channels'],filename='aac-extension-flag-'+name+'-invalid-synthetic.mp4');invalid.append(bad)
    missing=dict(cases[0],asc=bytes.fromhex(cases[0]['asc'])[:2].hex(),error='truncated or oversized bit field')
    missing['video']=video_fixture([missing],blob,channels=1,filename='aac-extension-flag-missing-future-bit-synthetic.mp4');invalid.append(missing)
    (DEST/'aac-extension-flag-packets.bin').write_bytes(blob)
    (DEST/'aac-extension-flag.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,packet_sha256=hashlib.sha256(blob).hexdigest(),qualification='extensionFlag=1 and extensionFlag3=0 must preserve baseline PCM; flag3=1 exact refusal'),indent=2)+'\n')
    print(len(cases),'extensionFlag scenarios; baseline, acceptance and invalid videos')
if __name__=='__main__':main()
