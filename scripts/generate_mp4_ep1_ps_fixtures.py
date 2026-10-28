"""Own PS movies with real separate EP1 ESC tracks. No production oracle run."""
import json
from generate_mp4_ep1_fixtures import SOURCE,make_movie,word
from generate_he_aac_packet_fixtures import box,boxes
from generate_mp4_ep1_empty_fixtures import fragmented
OUT=SOURCE/'mp4-ep1-ps'
def generate():
    parent=SOURCE/'aac-ep1';m=json.loads((parent/'manifest.json').read_bytes());units=(parent/'access-units.bin').read_bytes();plain=(parent/'logical-packets.bin').read_bytes();files={};cases=[]
    for base in m['ps_cases']:
        name=base['name'];seed=(parent/base['reference_video']).read_bytes();movie=make_movie(base,seed,units,frame_count=6,file_dependencies=True)
        files[base['core_reference']]=(parent/base['core_reference']).read_bytes()
        logical=bytearray();frames=[]
        for row in base['frames'][:6]:
            raw=plain[row['offset']:row['offset']+row['bytes']];frames.append(dict(offset=len(logical),bytes=len(raw)));logical.extend(raw)
        files[name+'.bin']=bytes(logical)
        for mode in ['indexed','fragmented','edits']:
            video=name+'-'+mode+'.mp4'
            files[video]=movie if mode=='indexed'else fragmented(movie,'per-row')if mode=='fragmented'else make_movie(base,seed,units,'edits',frame_count=6,file_dependencies=True)
            cases.append(dict(name=name+'-'+mode,video=video,signal=base['signal'],rate=base['rate'],channels=2,n=base['n'],mode=mode,plain_asc=base['plain_asc'],asc=base['asc'],core_reference=base['core_reference'],payloads=base['payloads'],logical=name+'.bin',frames=frames))
    ld_cases=[]
    for base in m['cases']:
        if not base['name'].startswith('eld-'):continue
        name=base['name'];seed=(parent/base['reference_video']).read_bytes()
        movie=make_movie(base,seed,units,allow_empty=True,frame_count=6,file_dependencies=True)
        golden=name+'-file.f32le';files[golden]=(parent/base['golden']).read_bytes()[:6*base['container_frame_samples']*base['channels']*4]
        for mode in ['indexed','fragmented']:
            video=name+'-file-'+mode+'.mp4';files[video]=movie if mode=='indexed' else fragmented(movie,'per-row')
            ld_cases.append(dict(name=name+'-file-'+mode,video=video,golden=golden,channels=base['channels'],rate=base['rate']))
    assert len(ld_cases)==4
    assert len(cases)==72
    seed=files[cases[0]['video']];bad=[]
    for mode,error in [('length','invalid MP4 dpnd table length'),('zero','zero MP4 dependency track ID'),('duplicate','duplicate MP4 singleton box'),('missing','EP1 MP4 missing dependency track'),('self','EP1 MP4 self dependency'),('multiple','EP1 MP4 multiple dependency parents are not supported')]:
        changed=False;child_id=0
        def rewrite(tag,body):
            nonlocal changed,child_id
            if tag in [b'moov',b'trak',b'tref']:
                children=[]
                for t,p in boxes(body):
                    if t==b'dpnd' and not changed:
                        changed=True
                        if mode=='length':p=p[:3]
                        elif mode=='zero':p=word(0)
                        elif mode=='missing':p=word(65535)
                        elif mode=='self':
                            # The containing trak identifies the child, independently of its ESDS.
                            p=word(child_id)
                        elif mode=='multiple':p+=p
                        children.append(box(t,p))
                        if mode=='duplicate':children.append(box(t,p))
                    else:
                        if t==b'trak':
                            tkhd=next(v for k,v in boxes(p) if k==b'tkhd');child_id=int.from_bytes(tkhd[12:16],'big')
                        children.append(rewrite(t,p))
                body=b''.join(children)
            return box(tag,body)
        data=b''.join(rewrite(t,p)for t,p in boxes(seed));assert changed
        video='invalid-dpnd-'+mode+'.mp4';files[video]=data;bad.append(dict(video=video,error=error,parser=mode in ['length','zero','duplicate']))
    files['manifest.json']=(json.dumps(dict(cases=cases,ld_cases=ld_cases,malformed=bad,provenance='Own ISO MP4 dpnd bindings with stored ES_ID zero, AVC video, ER-LC/LTP core and SBR/PS payloads with real EP1 ESC tracks. Raw unprotected AUs qualify transport equivalence; independent core PCM permits a separate shared PS-stage crosscheck, not an independent PS DSP oracle. 960/1024, 24/48 kHz, explicit/sync/implicit signals, six frames with changed headers. No foreign codec, private media, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
