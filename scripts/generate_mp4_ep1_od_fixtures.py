"""Own same-ASC EP1 membership from canonical MPEG-4 OD events, offline."""
import json
from generate_mp4_ep1_fixtures import SOURCE,word
from generate_mp4_ep1_epochs_fixtures import eld_control
from generate_mp4_ep1_active_chain_fixtures import streams
from generate_aac_esds_stream_fixtures import descriptor
from generate_he_aac_packet_fixtures import box,boxes,packed,field
OUT=SOURCE/'mp4-ep1-od'
def half(v):return v.to_bytes(2,'big')
# Expandable descriptor sizes may use all four bytes; wide updates then also
# exercise multi-byte enclosing lengths, including inside unaligned ESDUpdate.
def ref(i):return b'\x0f\x80\x80\x80\x02'+half(i)
def update(indices,object_id=2):
    return descriptor(1,descriptor(17,half((object_id<<6)|31)+b''.join(ref(i)for i in indices[::-1])))
def add(indices,object_id=2):
    bits=field(object_id,10)+''.join(''.join(f'{v:08b}'for v in ref(i))for i in indices)
    return descriptor(3,packed(bits))
def remove(indices):return descriptor(7,half(2<<6)+b''.join(half(i)for i in indices))
def clear():return descriptor(2,packed(field(2,10)))
def author(control,n,form,variant=None,unused=False):
    tracks=streams(control);width=len(tracks);ids=list(range(2,width+2));mapping=ids[1:]+ids[:1]
    indices=[mapping.index(i)+1 for i in ids];root=indices[:1];children=indices[1:]
    events=[update(root),update(indices),update(root)]if form=='snapshots'else[update(root),add(children),remove(children)]if form=='deltas'else[update(root),clear()+update(indices),clear()+update(root)]
    if variant=='ref-zero':events[1]=update([0])
    if variant=='duplicate':events[1]=update(indices+root)
    if variant=='gap':events[1]=update(indices[:2]+indices[3:])
    if variant=='late-command':events[1]=clear()+update(indices)+b'\xff\x00'
    if variant=='command-length':events[1]=b'\x01\x80\x80\x80\x80\x00'
    if variant=='unknown-object':events[1]=add(children,3)
    if variant=='multiple-objects':events[1]=update(indices)+update(root,3)
    if variant=='update-padding':events[1]=add(children)[:-1]+bytes([add(children)[-1]|1])
    parts=list(boxes(control));prefix=b''.join(box(t,p)for t,p in parts[:-1]);payload=bytearray();out=[]
    for ordinal,track in enumerate(tracks):
        frames=list(range(12))if ordinal==0 or unused else list(range(4,8))
        if variant=='missing'and ordinal==width-1:frames=frames[:-1]
        raw=[];pads=[]
        for i in frames:
            selected=range(width)if ordinal==0 and(i<4 or i>=8)else[ordinal]
            bits=''.join(''.join(f'{v:08b}'for v in tracks[j]['raw'][i])[:len(tracks[j]['raw'][i])*8-tracks[j]['pads'][i]]for j in selected)
            data=bytearray(packed(bits));pad=-len(bits)%8
            if pad:data[-1]|=(1<<pad)-1
            raw.append(data);pads.append(pad)
        scale=int.from_bytes(dict(boxes(dict(boxes(track['body']))[b'mdia']))[b'mdhd'][12:16],'big');factor=scale//24000;offset=len(prefix)+8+len(payload);payload.extend(b''.join(raw));count=len(raw)
        tables={b'stsz':word(0,0,count)+word(*map(len,raw)),b'stts':word(0,1,count,n*factor),b'stsc':word(0,1,1,count,1),b'stco':word(0,1,offset),b'padb':word(0,count)+bytes((pads[i]<<4)|(pads[i+1]if i+1<count else 0)for i in range(0,count,2))}
        if ordinal and not unused:tables[b'ctts']=word(0,1,count,4*n*factor)
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:
                p=b''.join(rewrite(k,v)for k,v in boxes(p)if k!=b'ctts')
                if t==b'stbl'and ordinal and not unused:p+=box(b'ctts',tables[b'ctts'])
            elif t in tables:p=tables[t]
            elif t==b'mdhd':p=p[:16]+word(count*n*factor)+p[20:]
            elif t==b'tkhd':p=p[:12]+word(ordinal+2)+p[16:20]+word((12 if ordinal==0 or unused else 8)*n)+p[24:]
            return box(t,p)
        body=b''.join(rewrite(t,p)for t,p in boxes(track['body'])if t!=b'tref')
        if ordinal:body+=box(b'tref',box(b'dpnd',word(ordinal+1)))
        out.append(box(b'trak',body))
    od_id=65536 if variant=='od-track-id'else 100
    first=dict(boxes(tracks[0]['body']));md=dict(boxes(first[b'mdia']));dinf=dict(boxes(md[b'minf']))[b'dinf']
    od_offset=len(prefix)+8+len(payload);payload.extend(b''.join(events));count=len(events)
    esds=box(b'esds',word(0)+descriptor(3,(half(1)+bytes(1)if variant=='od-esid'else bytes(3))+descriptor(4,bytes([0x40 if variant=='od-object-type'else 1,5])+bytes(11))+descriptor(6,bytes([1 if variant=='od-sl'else 2]))))
    stbl=box(b'stbl',box(b'stsd',word(0,1)+box(b'zzzz'if variant=='od-codec'else b'mp4s',bytes(6)+half(1)+esds))+box(b'stts',word(0,1,count,12*n))+box(b'stsc',word(0,1,1,count,1))+box(b'stsz',word(0,0,count)+word(*map(len,events)))+box(b'stco',word(0,1,od_offset)))
    tkhd=first[b'tkhd'];tkhd=tkhd[:12]+word(od_id)+tkhd[16:20]+word(12*n)+tkhd[24:36]+bytes(2)+tkhd[38:]
    mdhd=md[b'mdhd'];mdhd=mdhd[:12]+word(72000,36*n)+mdhd[20:]
    mpod=word(*(mapping if variant!='mpod-zero'else[0]+mapping[1:]))
    if variant=='mpod-length':mpod=mpod[:-1]
    od=box(b'trak',box(b'tkhd',tkhd)+box(b'tref',box(b'mpod',mpod))+box(b'mdia',box(b'mdhd',mdhd)+box(b'hdlr',word(0,0)+b'odsm'+bytes(12)+b'ObjectDescriptor\x00')+box(b'minf',box(b'nmhd',word(0))+box(b'dinf',dinf)+stbl)))
    moov=[];oldscale=int.from_bytes(dict(boxes(parts[-1][1]))[b'mvhd'][12:16],'big')
    for t,p in boxes(parts[-1][1]):
        if t==b'trak':
            mdia=dict(boxes(dict(boxes(p))[b'mdia']))
            if mdia[b'hdlr'][8:12]==b'soun':continue
            p=b''.join(box(k,v[:20]+word(int.from_bytes(v[20:24],'big')*24000//oldscale)+v[24:])if k==b'tkhd'else box(k,v)for k,v in boxes(p))
        if t==b'mvhd':p=p[:12]+word(24000,max(int.from_bytes(p[16:20],'big')*24000//oldscale,12*n))+p[20:-4]+word(od_id+1)
        moov.append(box(t,p))
    iod=half((1<<6)|(14 if variant=='iods-reserved'else 15))+bytes([254])*5+descriptor(14,word(od_id))
    if variant=='iods-url':
        url=b'urn:fvid:synthetic';iod=half((1<<6)|32|15)+bytes([len(url)])+url
    if variant!='no-iods':moov.append(box(b'iods',word(0)+descriptor(16,iod)))
    if variant=='duplicate-iods':moov.append(box(b'iods',word(0)+descriptor(16,iod)))
    return prefix+box(b'mdat',payload)+box(b'moov',b''.join(moov+out+[od])),events,mapping

def fragmented(source,n,hybrid=False):
    parts=list(boxes(source));assert parts[-1][0]==b'moov';prefix=b''.join(box(t,p)for t,p in parts[:-1]);moov=[];tracks=[];defaults=[]
    for tag,body in boxes(parts[-1][1]):
        if tag!=b'trak':moov.append(box(tag,body));continue
        children=dict(boxes(body));md=dict(boxes(children[b'mdia']));handler=md[b'hdlr'][8:12]
        if handler not in [b'soun',b'odsm']:moov.append(box(tag,body));continue
        ident=int.from_bytes(children[b'tkhd'][12:16],'big');tables=dict(boxes(dict(boxes(md[b'minf']))[b'stbl']));count=int.from_bytes(tables[b'stsz'][8:12],'big');offset=int.from_bytes(tables[b'stco'][8:12],'big');raw=[]
        sizes=[int.from_bytes(tables[b'stsz'][12+4*i:16+4*i],'big')for i in range(count)]
        for size in sizes:raw.append(source[offset:offset+size]);offset+=size
        pads=[tables[b'padb'][8+i//2]>>(4 if i%2==0 else 0)&7 for i in range(count)]if b'padb'in tables else[0]*count
        step=int.from_bytes(tables[b'stts'][12:16],'big');cts=int.from_bytes(tables[b'ctts'][12:16],'big')if b'ctts'in tables else 0
        phases={i:dict(raw=raw[i*4:i*4+4],pads=pads[i*4:i*4+4],dts=i*4*step)for i in range(3)}if handler==b'soun'and count==12 else{i:dict(raw=[raw[i]],pads=[0],dts=i*step)for i in range(count)}if handler==b'odsm'else{1:dict(raw=raw,pads=pads,dts=0)}
        tracks.append(dict(id=ident,step=step,cts=cts,phases=phases));defaults.append(box(b'trex',word(0,ident,1,7,7,0)))
        initial=phases.get(0)if hybrid else None
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:p=b''.join(rewrite(k,v)for k,v in boxes(p))
            elif t in [b'stsz',b'stts',b'stsc',b'stco',b'ctts',b'padb']:
                if initial is None:p=word(0,0,0)if t==b'stsz'else word(0,0)
                else:
                    qty=len(initial['raw'])
                    if t==b'stsz':p=word(0,0,qty)+word(*map(len,initial['raw']))
                    elif t==b'stts':p=word(0,1,qty,step)
                    elif t==b'stsc':p=word(0,1,1,qty,1)
                    elif t==b'stco':p=word(0,1)+tables[b'stco'][8:12]
                    elif t==b'ctts':p=word(0,1,qty,cts)
                    elif t==b'padb':p=word(0,qty)+bytes((initial['pads'][i]<<4)|(initial['pads'][i+1]if i+1<qty else 0)for i in range(0,qty,2))
            elif t==b'mdhd':p=p[:16]+word(len(initial['raw'])*step if initial else 0)+p[20:]
            return box(t,p)
        moov.append(box(tag,b''.join(rewrite(t,p)for t,p in boxes(body))))
    result=prefix+box(b'moov',b''.join(moov)+box(b'mvex',b''.join(defaults)))
    for epoch in range(1 if hybrid else 0,1+max(max(t['phases'])for t in tracks)):
        active=[(t,t['phases'][epoch])for t in tracks if epoch in t['phases']]
        def moof(base):
            offset=base;traf=[]
            for track,row in active:
                entries=b''.join(word(len(raw),pad<<17,track['cts'])for raw,pad in zip(row['raw'],row['pads']))
                tfhd=box(b'tfhd',word(0x02000a,track['id'],1,track['step']))
                tfdt=box(b'tfdt',word(0x01000000,row['dts']>>32,row['dts']&0xffffffff))
                traf.append(box(b'traf',tfhd+tfdt+box(b'trun',word(0xe01,len(row['raw']),offset)+entries)));offset+=sum(map(len,row['raw']))
            return box(b'moof',box(b'mfhd',word(0,epoch+1))+b''.join(traf))
        probe=moof(0);result+=moof(len(probe)+8)+box(b'mdat',b''.join(raw for _,row in active for raw in row['raw']))
    return result

def generate():
    files={};cases=[];bad=[]
    for n in [480,512]:
      for cfg in [1,7]:
        control,_,width,channels=eld_control(n,cfg,frame_count=12);stem=f'n{n}-c{cfg}'
        files[stem+'-control.mp4']=control
        for form in ['snapshots','deltas','replace']:
            name=stem+'-'+form
            for unused in [False,True]:
                movie,events,refs=author(control,n,form,unused=unused)
                for mode in ['indexed','fragmented','hybrid']:
                    label=name+('-unused'if unused else'')+'-'+mode;video=label+'.mp4';files[video]=movie if mode=='indexed'else fragmented(movie,n,mode=='hybrid')
                    cases.append(dict(name=label,video=video,control=stem+'-control.mp4',golden=f'../aac-eld-esc-sbr/n{n}-c{cfg}-x2-crc1.f32le',n=n,rate=48000,channels=channels,frames=12,active=[1,width,1],od_refs=refs,od_packets=[p.hex()for p in events],form=form,mode=mode,unused=unused,edges=[8*n,16*n]))
            video=name+'-indexed.mp4'
            variants=[('late-command','unsupported MP4 OD command',True,4),('ref-zero','MP4 OD reference index out of range',True,4),('duplicate','duplicate or zero MP4 OD ES reference',True,4),('command-length','MP4 OD descriptor length exceeds four bytes',True,4),('unknown-object','MP4 ESDUpdate targets unknown object',True,4),('multiple-objects','MP4 OD root belongs to multiple objects',False,4),('gap','EP1 MP4 OD active dependency chain has a gap',False,4),('missing','missing simultaneous ESC access unit',False,7),('no-iods','missing simultaneous ESC access unit',False,0),('update-padding','nonzero MP4 OD command alignment',True,4)]
            header_variants=[('mpod-length','invalid MP4 mpod reference table',True,0),('mpod-zero','zero MP4 mpod track ID',True,0),('iods-url','unsupported MP4 OD identifier or URL',True,0),('iods-reserved','invalid MP4 IOD reserved bits',True,0),('duplicate-iods','duplicate MP4 singleton box',True,0),('od-sl','MP4 OD requires self-contained SL configuration 2',True,0),('od-object-type','unsupported MP4 OD decoder config',True,0),('od-esid','MP4 OD stream requires canonical ES binding',True,0),('od-track-id','MP4 OD elementary track ID exceeds ES identifier range',True,0),('od-codec','MP4 referenced OD stream codec is not implemented',True,0)]if n==480 and cfg==7 and form=='snapshots'else[]
            for variant,error,parser,at in variants+header_variants:
                broken=name+'-'+variant+'.mp4';data,packets,_=author(control,n,form,variant);files[broken]=data;bad.append(dict(video=broken,good=video,error=error,parser=parser,at=at,n=n,width=width,variant=variant,packet=packets[1].hex(),od_refs=refs,command=not any(v[0]==variant for v in header_variants)))
    assert len(cases)==72
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,provenance='Own twelve-frame LD-SBR scalar/CLDFB reference; constant ASC with 1/full/1 EP1 membership selected by file ObjectDescriptorUpdate/ESDUpdate/ESRemoveRef/ObjectDescriptorRemove events. Canonical iods ES_ID_Inc to OD track, mpod refs, MP4_OD/ES_ID_Ref commands; non-byte ESDUpdate and 72-kHz OD clock. No private media, foreign codec, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
