"""Own ordinary/EP1/ordinary AAC movies; no codec processes or private media."""
import json
from generate_mp4_ep1_fixtures import SOURCE,word,make_movie
from generate_he_aac_packet_fixtures import box,boxes
from generate_aac_esds_stream_fixtures import descriptor
from generate_mp4_ep1_epochs_fixtures import fragmented_epochs,splice
OUT=SOURCE/'mp4-ep1-transitions'
def convert(source,asc,plain,trim=False,middle_plain=False,missing=False,empty_aux=False):
    parts=list(boxes(source));assert parts[-1][0]==b'moov'
    prefix=b''.join(box(t,p)for t,p in parts[:-1]);payload=bytearray();moov=[];dropped=False;defaults=[];fragment_root=None
    for tag,body in boxes(parts[-1][1]):
        if tag!=b'trak':moov.append(box(tag,body));continue
        children=dict(boxes(body));md=dict(boxes(children[b'mdia']))
        if md[b'hdlr'][8:12]!=b'soun':moov.append(box(tag,body));continue
        root=b'tref'not in children;tables=dict(boxes(dict(boxes(md[b'minf']))[b'stbl']));sizes=[int.from_bytes(tables[b'stsz'][12+4*i:16+4*i],'big')for i in range(18)]
        padding=[tables[b'padb'][8+i//2]>>(4 if i%2==0 else 0)&7 for i in range(18)];rows=[];steps=[]
        for epoch in range(3):
            offset=int.from_bytes(tables[b'stco'][8+4*epoch:12+4*epoch],'big');raw=[]
            for size in sizes[epoch*6:epoch*6+6]:raw.append(source[offset:offset+size]);offset+=size
            step=int.from_bytes(tables[b'stts'][12+8*epoch:16+8*epoch],'big');steps.append(step)
            ordinary=(epoch==1)if middle_plain else(epoch!=1)
            if root and ordinary:raw=plain;padding[epoch*6:epoch*6+6]=[0]*6
            if trim and not root and ordinary:continue
            pads=padding[epoch*6:epoch*6+6]
            if missing and not root and not ordinary and not dropped:
                raw=raw[:-1];pads=pads[:-1];dropped=True
            prior=sum(len(r['raw'])*r['step']for r in rows)
            rows.append(dict(raw=raw,pads=pads,step=step,description=2 if epoch==1 else 1,offset=6*sum(steps[:epoch])-prior))
        if empty_aux and not root:rows=[]
        offsets=[];newpads=[];newsizes=[]
        for row in rows:
            offsets.append(len(prefix)+8+len(payload));payload.extend(b''.join(row['raw']));newpads.extend(row['pads']);newsizes.extend(map(len,row['raw']))
        track_id=int.from_bytes(children[b'tkhd'][12:16],'big');defaults.append(box(b'trex',word(0,track_id,1,0,0,0)))
        if root:fragment_root=(track_id,rows[-1]['raw'][-1],rows[-1]['pads'][-1],rows[-1]['step'],sum(len(r['raw'])*r['step']for r in rows))
        count=len(newsizes);stsd=tables[b'stsd']
        if root:
            entries=list(boxes(stsd[8:]));esds=bytes(4)+descriptor(3,bytes(3)+descriptor(4,b'\x40\x14'+bytes(11)+descriptor(5,asc))+descriptor(6,b'\x02'))
            active=1 if middle_plain else 0;entries[active]=(b'mp4a',entries[active][1][:28]+box(b'esds',esds));stsd=word(0,2)+b''.join(box(t,p)for t,p in entries)
        replacements={b'stsd':stsd,b'stts':word(0,len(rows))+b''.join(word(len(r['raw']),r['step'])for r in rows),b'stsc':word(0,len(rows))+b''.join(word(i+1,len(r['raw']),r['description'])for i,r in enumerate(rows)),b'stco':word(0,len(offsets))+word(*offsets),b'stsz':word(0,0,count)+word(*newsizes),b'padb':word(0,count)+bytes((newpads[i]<<4)|(newpads[i+1]if i+1<count else 0)for i in range(0,count,2))}
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:
                p=b''.join(rewrite(k,v)for k,v in boxes(p)if k!=b'ctts')
                if t==b'stbl'and not root and (trim or missing):p+=box(b'ctts',word(0,len(rows))+b''.join(word(len(r['raw']),r['offset'])for r in rows))
            elif t in replacements:p=replacements[t]
            return box(t,p)
        moov.append(box(tag,b''.join(rewrite(t,p)for t,p in boxes(body))))
    if not empty_aux:return prefix+box(b'mdat',payload)+box(b'moov',b''.join(moov))
    # An actual root fragment permits empty auxiliary indexes, as in a live
    # hybrid movie whose dependent streams never supplied their promised AUs.
    track_id,raw,pad,step,start=fragment_root
    def moof(offset):
        return box(b'moof',box(b'mfhd',word(0,1))+box(b'traf',box(b'tfhd',word(0x02000a,track_id,1,step))+box(b'tfdt',word(0x01000000,start>>32,start&0xffffffff))+box(b'trun',word(0x601,1,offset,len(raw),pad<<17))))
    probe=moof(0)
    return prefix+box(b'mdat',payload)+box(b'moov',b''.join(moov)+box(b'mvex',b''.join(defaults)))+moof(len(probe)+8)+box(b'mdat',raw)
def ordinary_control(case,n):
    if case['name'].startswith('ps-'):
        directory=SOURCE/'aac-ep1';m=json.loads((directory/'manifest.json').read_bytes());aot=case['name'].split('-')[1];base=next(c for c in m['ps_cases']if c['name']==f"sbr-ps-{aot}-{n}-48000-{case.get('signal','explicit')}");blob=(directory/'logical-packets.bin').read_bytes();packets=[blob[r['offset']:r['offset']+r['bytes']]for r in base['frames'][:6]]
    else:
        directory=SOURCE/'aac-eld-esc-sbr';m=json.loads((directory/'manifest.json').read_bytes());configuration=case['name'].split('-')[2];base=next(c for c in m['cases']if c['name']==f'n{n}-{configuration}-x2-crc1');blob=(directory/'plain-packets.bin').read_bytes();packets=[blob[r['plain_offset']:r['plain_offset']+r['plain_bytes']]for r in base['frames'][:6]]
    return bytes.fromhex(base['plain_asc']),packets

def generate():
    previous=SOURCE/'mp4-ep1-epochs';manifest=json.loads((previous/'manifest.json').read_bytes());files={};cases=[];bad=[]
    seeds=[]
    for case in manifest['cases'][:4]:
        seed=(previous/case['video']).read_bytes();seeds.append((case,seed,case['video'],False))
        if not case['name'].startswith('ps-'):continue
        directory=SOURCE/'aac-ep1';parent=json.loads((directory/'manifest.json').read_bytes());units=(directory/'access-units.bin').read_bytes();aot=case['name'].split('-')[1]
        for signal in ['sync','implicit']:
            bases=[next(c for c in parent['ps_cases']if c['name']==f'sbr-ps-{aot}-{n}-48000-{signal}')for n in [960,1024]]
            movies=[make_movie(b,(directory/b['reference_video']).read_bytes(),units,frame_count=6,file_dependencies=True)for b in bases]
            alternate=dict(case,name=case['name']+'-'+signal,signal=signal);control=alternate['name']+'-all-ep1.mp4';files[control]=splice(movies);seeds.append((alternate,files[control],control,True))
    for case,seed,control,local_control in seeds:
        for reverse in [False,True]:
            n=(1024 if reverse else 960)if case['name'].startswith('ps-')else(512 if reverse else 480)
            asc,packets=ordinary_control(case,n);movie=convert(seed,asc,packets,middle_plain=reverse)
            name=case['name']+('-ep1-ordinary-ep1'if reverse else'-ordinary-ep1-ordinary')
            for mode in ['indexed','fragmented','hybrid','sparse-aux']:
                video=name+'-'+mode+'.mp4';files[video]=convert(seed,asc,packets,True,reverse)if mode=='sparse-aux'else fragmented_epochs(movie,mode=='hybrid')if mode in ['fragmented','hybrid']else movie
                cases.append(dict(name=name+'-'+mode,video=video,mode=mode,control=control,local_control=local_control,reverse=reverse,channels=case['channels'],streams=case['streams'],edges=case['edges'],frames=18,signal=case.get('signal','explicit')if case['name'].startswith('ps-')else'ld-sbr'))
            malformed=name+'-missing-active.mp4';files[malformed]=convert(seed,asc,packets,middle_plain=reverse,missing=True)
            bad.append(dict(video=malformed,sample=5 if reverse else 11,error='EP1 MP4 missing simultaneous ESC access unit'))
            empty=name+'-empty-aux.mp4';files[empty]=convert(seed,asc,packets,middle_plain=reverse,empty_aux=True)
            bad.append(dict(video=empty,parser=True,error='MP4 track states no samples'))
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,provenance='Own ordinary/EP1/ordinary and EP1/ordinary/EP1 ASC epochs; complete logical AUs on root in ordinary epochs, separate ESCs otherwise. Canonical dpnd, optional missing inactive auxiliary AUs; missing active AUs refuse. Same independent LD PCM and fresh PS epoch controls. No private media or foreign codec.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
