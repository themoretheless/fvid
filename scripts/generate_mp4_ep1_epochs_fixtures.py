"""Own EP1 MP4 description epochs, offline and without codec processes."""
import json
from generate_mp4_ep1_fixtures import SOURCE,make_movie,word
from generate_he_aac_packet_fixtures import box,boxes
OUT=SOURCE/'mp4-ep1-epochs'

def splice(movies, mismatch=False):
    parsed=[]
    for movie in movies:
        moov=dict(boxes(movie))[b'moov'];tracks=[]
        for tag,body in boxes(moov):
            if tag!=b'trak':continue
            parts=dict(boxes(body));md=dict(boxes(parts[b'mdia']))
            if md[b'hdlr'][8:12]!=b'soun':continue
            tables=dict(boxes(dict(boxes(md[b'minf']))[b'stbl']));count=int.from_bytes(tables[b'stsz'][8:12],'big');offset=int.from_bytes(tables[b'stco'][8:12],'big');packets=[]
            for i in range(count):
                size=int.from_bytes(tables[b'stsz'][12+i*4:16+i*4],'big');packets.append(movie[offset:offset+size]);offset+=size
            padding=[tables[b'padb'][8+i//2]>>(4 if i%2==0 else 0)&7 for i in range(count)]
            tracks.append(dict(body=body,tables=tables,packets=packets,padding=padding,step=int.from_bytes(tables[b'stts'][12:16],'big')))
        parsed.append(tracks)
    assert len(parsed[0])==len(parsed[1])
    prefix=b''.join(box(t,p)for t,p in boxes(movies[0])if t!=b'moov');payload=bytearray();result=[]
    oldmoov=dict(boxes(movies[0]))[b'moov']
    for tag,body in boxes(oldmoov):
        if tag!=b'trak':result.append(box(tag,body));continue
        md=dict(boxes(dict(boxes(body))[b'mdia']))
        if md[b'hdlr'][8:12]!=b'soun':result.append(box(tag,body));continue
        ordinal=next(i for i,t in enumerate(parsed[0])if t['body']==body)
        rows=[parsed[0][ordinal],parsed[1][ordinal],parsed[0][ordinal]]
        chunks=[];sizes=[];padding=[]
        for row in rows:
            chunks.append(len(prefix)+8+len(payload));payload.extend(b''.join(row['packets']));sizes.extend(map(len,row['packets']));padding.extend(row['padding'])
        entries=[r['tables'][b'stsd'][8:] for r in rows[:2]]
        # One child deliberately keeps the old ASC in its middle epoch.
        if mismatch and ordinal==0:entries[1]=entries[0]
        count=sum(len(r['packets'])for r in rows);duration=sum(len(r['packets'])*r['step']for r in rows)
        tables={b'stsd':word(0,2)+b''.join(entries),b'stts':word(0,3)+b''.join(word(len(r['packets']),r['step'])for r in rows),b'stsc':word(0,3)+b''.join(word(i+1,len(r['packets']),2 if i==1 else 1)for i,r in enumerate(rows)),b'stco':word(0,3)+word(*chunks),b'stsz':word(0,0,count)+word(*sizes),b'padb':word(0,count)+bytes((padding[i]<<4)|(padding[i+1]if i+1<count else 0)for i in range(0,count,2))}
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:p=b''.join(rewrite(k,v)for k,v in boxes(p))
            elif t in tables:p=tables[t]
            elif t==b'mdhd':p=p[:16]+word(duration)+p[20:]
            return box(t,p)
        result.append(box(tag,b''.join(rewrite(t,p)for t,p in boxes(body))))
    return prefix+box(b'mdat',payload)+box(b'moov',b''.join(result))

def fragmented_epochs(source,hybrid=False):
    parts=list(boxes(source));assert parts[-1][0]==b'moov'
    prefix=b''.join(box(t,p)for t,p in parts[:-1]);tracks=[];moov=[];defaults=[]
    for tag,body in boxes(parts[-1][1]):
        if tag!=b'trak':moov.append(box(tag,body));continue
        children=dict(boxes(body));md=dict(boxes(children[b'mdia']))
        if md[b'hdlr'][8:12]!=b'soun':moov.append(box(tag,body));continue
        track_id=int.from_bytes(children[b'tkhd'][12:16],'big');tables=dict(boxes(dict(boxes(md[b'minf']))[b'stbl']))
        count=int.from_bytes(tables[b'stsz'][8:12],'big');assert count==18
        sizes=[int.from_bytes(tables[b'stsz'][12+4*i:16+4*i],'big')for i in range(count)]
        pads=[tables[b'padb'][8+i//2]>>(4 if i%2==0 else 0)&7 for i in range(count)];epochs=[]
        for i in range(3):
            offset=int.from_bytes(tables[b'stco'][8+4*i:12+4*i],'big');raw=[]
            for size in sizes[6*i:6*i+6]:raw.append(source[offset:offset+size]);offset+=size
            step=int.from_bytes(tables[b'stts'][12+8*i:16+8*i],'big');description=int.from_bytes(tables[b'stsc'][16+12*i:20+12*i],'big')
            assert description==[1,2,1][i]
            epochs.append(dict(raw=raw,pads=pads[6*i:6*i+6],step=step,description=description))
        tracks.append(dict(id=track_id,epochs=epochs))
        # Deliberately distinct defaults; each fragment must use its explicit setup.
        defaults.append(box(b'trex',word(0,track_id,2,7,7,0)))
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:p=b''.join(rewrite(k,v)for k,v in boxes(p))
            elif t==b'stsd':pass
            elif t in [b'stts',b'stsc',b'stco',b'padb',b'stsz']:
                if not hybrid:p=word(0,0,0)if t==b'stsz' else word(0,0)
                elif t==b'stts':p=word(0,1,6,epochs[0]['step'])
                elif t==b'stsc':p=word(0,1,1,6,1)
                elif t==b'stco':p=word(0,1)+p[8:12]
                elif t==b'stsz':p=word(0,0,6)+word(*sizes[:6])
                elif t==b'padb':p=word(0,6)+p[8:11]
            elif t==b'mdhd':p=p[:16]+word(6*epochs[0]['step']if hybrid else 0)+p[20:]
            return box(t,p)
        moov.append(box(tag,b''.join(rewrite(t,p)for t,p in boxes(body))))
    result=prefix+box(b'moov',b''.join(moov)+box(b'mvex',b''.join(defaults)))
    for epoch in range(1 if hybrid else 0,3):
        def moof(base):
            offset=base;traf=[]
            for track in tracks:
                row=track['epochs'][epoch];start=sum(6*e['step']for e in track['epochs'][:epoch])
                tfhd=box(b'tfhd',word(0x02000a,track['id'],row['description'],row['step']))
                tfdt=box(b'tfdt',word(0x01000000,start>>32,start&0xffffffff))
                entries=b''.join(word(len(raw),pad<<17)for raw,pad in zip(row['raw'],row['pads']))
                trun=box(b'trun',word(0x601,6,offset)+entries)
                traf.append(box(b'traf',tfhd+tfdt+trun));offset+=sum(map(len,row['raw']))
            return box(b'moof',box(b'mfhd',word(0,epoch+1))+b''.join(traf))
        probe=moof(0);payload=b''.join(raw for track in tracks for raw in track['epochs'][epoch]['raw'])
        result+=moof(len(probe)+8)+box(b'mdat',payload)
    return result

def unused_first_description(source):
    from generate_aac_esds_stream_fixtures import descriptor
    # A valid ordinary AAC ASC, deliberately unused by every sample.
    esds=box(b'esds',bytes(4)+descriptor(3,bytes(3)+descriptor(4,b'\x40\x14'+bytes(11)+descriptor(5,b'\x12\x10'))+descriptor(6,b'\x02')))
    def rewrite(tag,p,audio=False):
        if tag==b'trak':
            md=dict(boxes(dict(boxes(p))[b'mdia']));audio=md[b'hdlr'][8:12]==b'soun'
        if tag in [b'moov',b'trak',b'mdia',b'minf',b'stbl',b'mvex',b'moof',b'traf']:
            p=b''.join(rewrite(t,v,audio)for t,v in boxes(p))
        elif tag==b'stsd' and audio:
            entries=list(boxes(p[8:]));unused=box(b'mp4a',entries[0][1][:28]+esds)
            p=word(0,len(entries)+1)+unused+p[8:]
        elif tag==b'stsc' and audio:
            count=int.from_bytes(p[4:8],'big');rows=[]
            for i in range(count):
                row=p[8+i*12:20+i*12];rows.append(row[:8]+word(int.from_bytes(row[8:12],'big')+1))
            p=p[:8]+b''.join(rows)
        elif tag==b'trex':p=p[:8]+word(int.from_bytes(p[8:12],'big')+1)+p[12:]
        elif tag==b'tfhd':
            flags=int.from_bytes(p[:4],'big');assert flags&2 and not flags&1
            p=p[:8]+word(int.from_bytes(p[8:12],'big')+1)+p[12:]
        return box(tag,p)
    return b''.join(rewrite(t,p)for t,p in boxes(source))

def eld_control(n,configuration,crc=True,dual=True):
    from generate_he_aac_packet_fixtures import packed,field,frequency
    from generate_aac_sbr_dsp_fixtures import header
    parent=SOURCE/'aac-eld-esc-sbr';m=json.loads((parent/'manifest.json').read_bytes())
    name=f'n{n}-c{configuration}-x{2 if dual else 1}-crc{int(crc)}';base=next(c for c in m['cases']if c['name']==name)
    info=(parent/'instances.bin').read_bytes();blob=bytearray();rows=[]
    for index,row in enumerate(base['frames'][:6]):
        parts=[]
        for p in row['instances']:
            bits=''.join(f'{v:08b}'for v in info[p['offset']:p['offset']+p['bytes']])[:p['bits']]
            parts.append(dict(category=p['category'],bits=bits))
        total=sum(len(p['bits'])for p in parts);logical=row.get('logical_bits',total)
        if total!=logical:parts[-1]['bits']=parts[-1]['bits'][:len(parts[-1]['bits'])-(total-logical)]
        counts={};units=[];parent_id=None
        for ordinal,part in enumerate(parts):
            cat=part['category'];occurrence=counts.get(cat,0);counts[cat]=occurrence+1;sid=cat*1024+occurrence*7+19
            bits=part['bits']+('0'*(-logical%8)if ordinal+1==len(parts)else'');data=bytearray(packed(bits))
            if len(bits)%8:data[-1]|=(1<<(8-len(bits)%8))-1
            scale=1 if ordinal==0 else 1+ordinal%3
            units.append(dict(stream_id=sid,depends_on=parent_id,offset=len(blob),bytes=len(data),bits=len(bits),timescale=24000*scale,duration=n*scale));blob.extend(data);parent_id=sid
        rows.append(dict(units=units[1:]+units[:1]))
    groups={1:1,2:1,3:2,4:3,5:3,6:3,7:4}[configuration]
    setup=field(31,5)+field(7,6)+frequency(24000)+field(configuration,4)+field(n==480,1)+'000'+'1'+str(int(dual))+str(int(crc))+header(0,True)*groups+'0000'
    assert packed(setup+'00').hex()==base['plain_asc']
    seed=(parent/base['video']).read_bytes();fixture=dict(base,asc=packed(setup+'01').hex(),frames=rows)
    movie=make_movie(fixture,seed,blob,allow_empty=True,frame_count=6,file_dependencies=True)
    gold=(parent/(name+'.f32le')).read_bytes()[:6*base['container_frame_samples']*base['channels']*4]
    return movie,gold,len(rows[0]['units']),base['channels']

def generate():
    parent=SOURCE/'aac-ep1';m=json.loads((parent/'manifest.json').read_bytes());units=(parent/'access-units.bin').read_bytes();files={};cases=[]
    for aot in [17,19]:
        bases=[next(c for c in m['ps_cases']if c['name']==f'sbr-ps-aot{aot}-{n}-48000-explicit')for n in [960,1024]]
        movies=[make_movie(b,(parent/b['reference_video']).read_bytes(),units,frame_count=6,file_dependencies=True)for b in bases]
        name=f'ps-aot{aot}-960-1024-960';files[name+'.mp4']=splice(movies);files[name+'-mismatch.mp4']=splice(movies,True)
        cases.append(dict(name=name,video=name+'.mp4',malformed=name+'-mismatch.mp4',controls=[b['name']+'-indexed.mp4'for b in bases],frames=18,streams=4,channels=2,edges=[11520,23808]))
    for configuration in [1,7]:
        controls=[eld_control(n,configuration)for n in [480,512]];name=f'ld-sbr-c{configuration}-480-512-480'
        files[name+'.mp4']=splice([c[0]for c in controls]);files[name+'-mismatch.mp4']=splice([c[0]for c in controls],True)
        names=[]
        for i,(movie,gold,streams,channels)in enumerate(controls):
            control=name+f'-control-{i}.mp4';files[control]=movie;files[control+'.f32le']=gold;names.append(control)
        assert controls[0][2:]==controls[1][2:]
        cases.append(dict(name=name,video=name+'.mp4',malformed=name+'-mismatch.mp4',controls=names,local_controls=True,frames=18,streams=controls[0][2],channels=controls[0][3],edges=[5760,11904]))
    indexed=list(cases)
    for case in indexed:
        for mode in ['fragmented','hybrid']:
            name=case['name']+'-'+mode;video=name+'.mp4';malformed=name+'-mismatch.mp4'
            files[video]=fragmented_epochs(files[case['video']],mode=='hybrid')
            files[malformed]=fragmented_epochs(files[case['malformed']],mode=='hybrid')
            cases.append(dict(case,name=name,video=video,malformed=malformed,mode=mode,indexed=case['video']))
    for case in list(cases):
        name=case['name']+'-unused-first';video=name+'.mp4';malformed=name+'-mismatch.mp4'
        files[video]=unused_first_description(files[case['video']]);files[malformed]=unused_first_description(files[case['malformed']])
        cases.append(dict(case,name=name,video=video,malformed=malformed,unused_first=True,indexed=case.get('indexed',case['video'])))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own six-AU EP1 PS and LD-SBR controls spliced into 960/1024/960 and 480/512/480 sample-description epochs with canonical dpnd, exact padding and rational clocks. Controls reset each epoch; mismatch changes a dependent ASC only. No private media or foreign codec.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
