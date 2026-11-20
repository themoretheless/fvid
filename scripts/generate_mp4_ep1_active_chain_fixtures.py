"""Own mono/multichannel/mono separate ESC activity; static canonical dpnd."""
import json
from generate_mp4_ep1_fixtures import SOURCE,word
from generate_mp4_ep1_epochs_fixtures import eld_control
from generate_he_aac_packet_fixtures import box,boxes
OUT=SOURCE/'mp4-ep1-active-chain'
def streams(movie):
    indexed={}
    for tag,body in boxes(dict(boxes(movie))[b'moov']):
        if tag!=b'trak':continue
        children=dict(boxes(body));md=dict(boxes(children[b'mdia']))
        if md[b'hdlr'][8:12]!=b'soun':continue
        ident=int.from_bytes(children[b'tkhd'][12:16],'big');parent=None
        if b'tref'in children:parent=int.from_bytes(dict(boxes(children[b'tref']))[b'dpnd'],'big')
        tables=dict(boxes(dict(boxes(md[b'minf']))[b'stbl']));count=int.from_bytes(tables[b'stsz'][8:12],'big');assert count in [3,6,12,16]
        offset=int.from_bytes(tables[b'stco'][8:12],'big');raw=[]
        for i in range(count):
            size=int.from_bytes(tables[b'stsz'][12+4*i:16+4*i],'big');raw.append(movie[offset:offset+size]);offset+=size
        pads=[tables[b'padb'][8+i//2]>>(4 if i%2==0 else 0)&7 for i in range(count)]
        indexed[ident]=dict(body=body,parent=parent,raw=raw,pads=pads,entry=tables[b'stsd'][8:])
    ordered=[];parent=None
    while len(ordered)<len(indexed):
        found=[(i,t)for i,t in indexed.items()if t['parent']==parent];assert len(found)==1
        ident,track=found[0];ordered.append(track);parent=ident
    return ordered

def movie(controls,missing=False,unused=False,gap_entry=None):
    small,big=[streams(c)for c in controls];assert len(small)==4 and len(big)==27
    parts=list(boxes(controls[0]));original_movie_scale=int.from_bytes(dict(boxes(parts[-1][1]))[b'mvhd'][12:16],'big');movie_scale=24000;prefix=b''.join(box(t,p)for t,p in parts[:-1]);payload=bytearray();tracks=[];dropped=False
    for ordinal in range(len(big)):
        scale=24000*(1+ordinal%3);factor=scale//24000;rows=[]
        for epoch in range(3):
            if epoch!=1 and ordinal>=len(small):continue
            source=big[ordinal]if epoch==1 else small[ordinal];raw=source['raw'];pads=source['pads']
            if missing and ordinal==len(big)-1 and epoch==1:raw=raw[:-1];pads=pads[:-1];dropped=True
            rows.append(dict(raw=raw,pads=pads,step=(512 if epoch==1 else 480)*factor,description=(2 if epoch==1 else 1)if ordinal<len(small)else 1,start=[0,6*480,6*(480+512)][epoch]*factor))
        offsets=[];sizes=[];padding=[];ctts=[];dts=0
        for row in rows:
            offsets.append(len(prefix)+8+len(payload));payload.extend(b''.join(row['raw']));sizes.extend(map(len,row['raw']));padding.extend(row['pads']);ctts.append((len(row['raw']),row['start']-dts));dts+=len(row['raw'])*row['step']
        count=len(sizes);entries=[small[ordinal]['entry'],big[ordinal]['entry']]if ordinal<len(small)else[big[ordinal]['entry']]
        if unused and ordinal>=len(small):entries.append(small[0]['entry'])
        if gap_entry is not None and ordinal==len(small):
            entries=[gap_entry];ctts=[(c,o+factor)for c,o in ctts]
        tables={b'stsd':word(0,len(entries))+b''.join(entries),b'stts':word(0,len(rows))+b''.join(word(len(r['raw']),r['step'])for r in rows),b'stsc':word(0,len(rows))+b''.join(word(i+1,len(r['raw']),r['description'])for i,r in enumerate(rows)),b'stco':word(0,len(offsets))+word(*offsets),b'stsz':word(0,0,count)+word(*sizes),b'padb':word(0,count)+bytes((padding[i]<<4)|(padding[i+1]if i+1<count else 0)for i in range(0,count,2))}
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:
                p=b''.join(rewrite(k,v)for k,v in boxes(p)if k not in [b'ctts',b'padb'])
                if t==b'stbl':p+=box(b'padb',tables[b'padb'])+box(b'ctts',word(0,len(ctts))+b''.join(word(c,o)for c,o in ctts))
            elif t in tables:p=tables[t]
            elif t==b'tkhd':p=p[:12]+word(ordinal+2)+p[16:20]+word(((rows[-1]['start']+len(rows[-1]['raw'])*rows[-1]['step'])*movie_scale+scale-1)//scale)+p[24:]
            elif t==b'mdhd':p=p[:12]+word(scale,dts)+p[20:]
            return box(t,p)
        body=small[ordinal]['body']if ordinal<len(small)else big[ordinal]['body']
        rewritten=b''.join(rewrite(t,p)for t,p in boxes(body)if t!=b'tref')
        if ordinal:rewritten+=box(b'tref',box(b'dpnd',word(ordinal+1)))
        tracks.append(box(b'trak',rewritten))
    assert not missing or dropped
    moov=[]
    for tag,body in boxes(parts[-1][1]):
        if tag==b'trak':
            md=dict(boxes(dict(boxes(body))[b'mdia']))
            if md[b'hdlr'][8:12]==b'soun':continue
            assert b'edts'not in dict(boxes(body))
            body=b''.join(box(t,p[:20]+word(int.from_bytes(p[20:24],'big')*movie_scale//original_movie_scale)+p[24:])if t==b'tkhd'else box(t,p)for t,p in boxes(body))
        if tag==b'mvhd':body=body[:12]+word(movie_scale,6*(480+512+480))+body[20:-4]+word(len(big)+2)
        moov.append(box(tag,body))
    return prefix+box(b'mdat',payload)+box(b'moov',b''.join(moov+tracks))
def fragmented(source,hybrid=False):
    parts=list(boxes(source));assert parts[-1][0]==b'moov'
    prefix=b''.join(box(t,p)for t,p in parts[:-1]);moov=[];defaults=[];tracks=[]
    def runs(p):
        return [int.from_bytes(p[12+8*i:16+8*i],'big')for i in range(int.from_bytes(p[4:8],'big'))for _ in range(int.from_bytes(p[8+8*i:12+8*i],'big'))]
    for tag,body in boxes(parts[-1][1]):
        if tag!=b'trak':moov.append(box(tag,body));continue
        children=dict(boxes(body));md=dict(boxes(children[b'mdia']))
        if md[b'hdlr'][8:12]!=b'soun':moov.append(box(tag,body));continue
        tables=dict(boxes(dict(boxes(md[b'minf']))[b'stbl']));ident=int.from_bytes(children[b'tkhd'][12:16],'big');count=int.from_bytes(tables[b'stsz'][8:12],'big')
        sizes=[int.from_bytes(tables[b'stsz'][12+4*i:16+4*i],'big')for i in range(count)];pads=[tables[b'padb'][8+i//2]>>(4 if i%2==0 else 0)&7 for i in range(count)]
        duration=runs(tables[b'stts']);composition=runs(tables[b'ctts']);assert len(duration)==len(composition)==count
        chunks=int.from_bytes(tables[b'stco'][4:8],'big');assert chunks in [1,3]and int.from_bytes(tables[b'stsc'][4:8],'big')==chunks
        rows={};cursor=0;dts=0
        for i in range(chunks):
            row=tables[b'stsc'][8+12*i:20+12*i];assert int.from_bytes(row[:4],'big')==i+1
            qty=int.from_bytes(row[4:8],'big');desc=int.from_bytes(row[8:12],'big');offset=int.from_bytes(tables[b'stco'][8+4*i:12+4*i],'big');raw=[]
            for size in sizes[cursor:cursor+qty]:raw.append(source[offset:offset+size]);offset+=size
            step=duration[cursor];cts=composition[cursor];assert all(v==step for v in duration[cursor:cursor+qty])and all(v==cts for v in composition[cursor:cursor+qty])
            epoch=i if chunks==3 else 1
            rows[epoch]=dict(raw=raw,pads=pads[cursor:cursor+qty],description=desc,step=step,cts=cts,dts=dts)
            cursor+=qty;dts+=qty*step
        assert cursor==count;tracks.append(dict(id=ident,rows=rows))
        descriptions=int.from_bytes(tables[b'stsd'][4:8],'big');defaults.append(box(b'trex',word(0,ident,descriptions,7,7,0)))
        initial=rows.get(0)if hybrid else None
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:p=b''.join(rewrite(k,v)for k,v in boxes(p))
            elif t in [b'stsz',b'stts',b'stsc',b'stco',b'ctts',b'padb']:
                if initial is None:p=word(0,0,0)if t==b'stsz'else word(0,0)
                else:
                    qty=len(initial['raw'])
                    if t==b'stsz':p=word(0,0,qty)+word(*(len(v)for v in initial['raw']))
                    elif t==b'stts':p=word(0,1,qty,initial['step'])
                    elif t==b'stsc':p=word(0,1,1,qty,initial['description'])
                    elif t==b'stco':p=word(0,1)+p[8:12]
                    elif t==b'ctts':p=word(0,1,qty,initial['cts'])
                    elif t==b'padb':p=word(0,qty)+bytes((initial['pads'][i]<<4)|(initial['pads'][i+1]if i+1<qty else 0)for i in range(0,qty,2))
            elif t==b'mdhd':p=p[:16]+word(len(initial['raw'])*initial['step']if initial else 0)+p[20:]
            return box(t,p)
        moov.append(box(tag,b''.join(rewrite(t,p)for t,p in boxes(body))))
    result=prefix+box(b'moov',b''.join(moov)+box(b'mvex',b''.join(defaults)))
    for epoch in range(1 if hybrid else 0,3):
        active=[(t['id'],t['rows'][epoch])for t in tracks if epoch in t['rows']]
        assert len(active)==[4,27,4][epoch]
        def moof(base):
            offset=base;traf=[]
            for ident,row in active:
                tfhd=box(b'tfhd',word(0x02000a,ident,row['description'],row['step']))
                tfdt=box(b'tfdt',word(0x01000000,row['dts']>>32,row['dts']&0xffffffff))
                entries=b''.join(word(len(raw),pad<<17,row['cts'])for raw,pad in zip(row['raw'],row['pads']))
                trun=box(b'trun',word(0xe01,len(row['raw']),offset)+entries);traf.append(box(b'traf',tfhd+tfdt+trun));offset+=sum(map(len,row['raw']))
            return box(b'moof',box(b'mfhd',word(0,epoch+1))+b''.join(traf))
        probe=moof(0);payload=b''.join(raw for _,row in active for raw in row['raw'])
        result+=moof(len(probe)+8)+box(b'mdat',payload)
    return result

def generate():
    controls=[eld_control(480,1),eld_control(512,7)];files={}
    for i,(data,gold,_,_)in enumerate(controls):files[f'control-{i}.mp4']=data;files[f'control-{i}.f32le']=gold
    files['mono-eight-mono.mp4']=movie([c[0]for c in controls]);files['missing-active-tail.mp4']=movie([c[0]for c in controls],True)
    files['unused-tail-config.mp4']=movie([c[0]for c in controls],unused=True)
    extra=eld_control(512,1)[0];files['active-gap.mp4']=movie([c[0]for c in controls],gap_entry=streams(extra)[0]['entry'])
    base=dict(name='ld-sbr-mono-eight-mono',video='mono-eight-mono.mp4',equivalent='unused-tail-config.mp4',gap='active-gap.mp4',malformed='missing-active-tail.mp4',controls=['control-0.mp4','control-1.mp4'],golden=['control-0.f32le','control-1.f32le'],active=[4,27,4],channels=[1,8,1],frames=[5760,6144,5760],edges=[5760,11904],mode='indexed')
    cases=[base]
    for mode in ['fragmented','hybrid']:
        case=dict(base,name=base['name']+'-'+mode,mode=mode)
        for key in ['video','equivalent','gap','malformed']:
            filename=base[key][:-4]+'-'+mode+'.mp4';files[filename]=fragmented(files[base[key]],mode=='hybrid');case[key]=filename
        cases.append(case)
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own LD-SBR mono/8-channel/mono epochs; static canonical dpnd union, four-stream active prefix then 27 streams then four. Tail tracks only have middle-epoch AUs. Indexed, fragmented and hybrid storage with explicit descriptions, DTS/CTS, exact padding and inactive-traf omission. Independent scalar native PCM and rational per-track clocks. No private media, external codec, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
