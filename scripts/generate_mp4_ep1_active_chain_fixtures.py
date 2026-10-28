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
        tables=dict(boxes(dict(boxes(md[b'minf']))[b'stbl']));count=int.from_bytes(tables[b'stsz'][8:12],'big');assert count==6
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
def generate():
    controls=[eld_control(480,1),eld_control(512,7)];files={}
    for i,(data,gold,_,_)in enumerate(controls):files[f'control-{i}.mp4']=data;files[f'control-{i}.f32le']=gold
    files['mono-eight-mono.mp4']=movie([c[0]for c in controls]);files['missing-active-tail.mp4']=movie([c[0]for c in controls],True)
    files['unused-tail-config.mp4']=movie([c[0]for c in controls],unused=True)
    extra=eld_control(512,1)[0];files['active-gap.mp4']=movie([c[0]for c in controls],gap_entry=streams(extra)[0]['entry'])
    files['manifest.json']=(json.dumps(dict(cases=[dict(name='ld-sbr-mono-eight-mono',video='mono-eight-mono.mp4',equivalent='unused-tail-config.mp4',gap='active-gap.mp4',malformed='missing-active-tail.mp4',controls=['control-0.mp4','control-1.mp4'],golden=['control-0.f32le','control-1.f32le'],active=[4,27,4],channels=[1,8,1],frames=[5760,6144,5760],edges=[5760,11904])],provenance='Own LD-SBR mono/8-channel/mono epochs; static canonical dpnd union, four-stream active prefix then 27 streams then four. Tail tracks only have middle-epoch AUs. Independent scalar native PCM, exact bits and rational per-track clocks. No private media, external codec, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
