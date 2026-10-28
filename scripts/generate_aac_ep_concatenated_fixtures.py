"""Own class-wise multi-source-frame EP2 packets, PCM and short movies."""
import json
from pathlib import Path
from generate_aac_eld_fixtures import generate as eld,mkv
from generate_aac_er_ep_fixtures import prefix as er_prefix
from generate_aac_ep_multiclass_fixtures import chain,encode,crc,header,interleave,window,octet_stage
from generate_aac_sbr_dsp_fixtures import header as sbr_header
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-ep-concatenated'

def config(lengths,frames,family,kind,flagmode,escape=None):
    sets=[]
    for selected in range(3):
        count=len(lengths);order=list(range(count))
        if selected==1:order=order[1:]+order[:1]
        if selected==2:order.reverse()
        classes=[]
        for k,n in enumerate(lengths):
            fec=1 if family=='rs'or family=='mixed'and k in [0,count-1]else 0
            switch=([1,0,2,3][k%4]if family=='mixed'else [0,1,2,3][k%4])
            if fec and switch==2:switch=1
            joined=flagmode=='joined'or flagmode=='mixed'and k%2==0
            classes.append(dict(fec=fec,rate=(2+selected)if fec else (k*7+3+selected)%25,crc=([8,16,24,32][k%4]if fec else [4,8,12,16][k%4]),terminated=fec==0 and k==order[-1],interleave=switch,length=n,joined=joined))
        sets.append(dict(classes=classes,order=order))
    ep=field(3,8)+field(kind,2)+'001'+field(frames,3)
    for s in sets:
        ep+=field(len(s['classes']),6)
        for k,c in enumerate(s['classes']):
            escaped=escape if k==0 else None
            ep+=field(escaped=='length',1)+field(escaped=='rate',1)+field(escaped=='crc',1)+field(c['joined'],1)+field(c['fec'],2)
            if c['fec']==0:ep+=field(c['terminated'],1)
            if kind==2:ep+=field(c['interleave'],2)
            ep+='0'+field(15 if escaped=='length'else c['length'],4 if escaped=='length'else 16)
            if escaped!='rate':ep+=field(c['rate'],7 if c['fec']else 5)
            if escaped!='crc':ep+=field(17 if c['crc']==24 else 18 if c['crc']==32 else c['crc'],5)
        ep+='1'+''.join(field(k,6)for k in s['order'])
    return sets,ep+'1'+field(24,5)+field(16,5)

def protect(units,sets,kind,i,mutation=None):
    selected=i%3;s=sets[selected];classes=s['classes'];rows=[]
    messages=[''.join(f'{v:08b}'for v in raw)for raw in units]
    offsets=[];at=0
    for c in classes:offsets.append(at);at+=c['length']
    assert all(at==len(m)for m in messages)
    for k in s['order']:
        c=classes[k];a=offsets[k];parts=[m[a:a+c['length']]for m in messages]
        for message in [''.join(parts)]if c['joined']else parts:
            parity=crc(message,c['crc'])if message else ''
            rows.append(dict(c=c,message=message,payload=message+parity))
    if mutation=='crc':
        row=next(r for r in reversed(rows)if r['message']and r['c']['crc']);at=len(row['message']);row['payload']=row['payload'][:at]+str(1-int(row['payload'][at]))+row['payload'][at+1:]
    src=[j for j,r in enumerate(rows)if r['c']['fec']==0 and r['message']];coded=['']*len(rows)
    for j,value in zip(src,chain([rows[j]['payload']for j in src],[rows[j]['c']['rate']for j in src],[rows[j]['c']['terminated']for j in src])):coded[j]=value
    for j,r in enumerate(rows):
        if r['c']['fec']and r['message']:coded[j]=''.join(f'{v:08b}'for v in encode(packed(r['payload']),r['c']['rate']))
    if mutation=='truncated':
        j=next(j for j in reversed(range(len(rows)))if len(coded[j])>=8);coded[j]=coded[j][:-8]
    selector=header(field(selected,2));h=header('000');stuff=(-len(selector)-len(h)-sum(map(len,coded)))%8;h=header(field(stuff,3))
    if kind==0:body=''.join(coded)
    else:
        body=''.join(x for x,r in zip(coded,rows)if kind==2 and r['c']['interleave']==3)
        for x,r in reversed(list(zip(coded,rows))):
            c=r['c'];switch=c['interleave']
            if x and(kind==1 or switch in [1,2]):body=''.join(octet_stage(list(x),list(body)))if c['fec']else interleave(x,body,28 if kind==1 or switch==2 else len(x))
        if kind==2:body+=''.join(x for x,r in zip(coded,rows)if r['c']['interleave']==0)
    bits=selector+h+body+'0'*stuff if kind==0 else interleave(selector,interleave(h,body+'0'*stuff,len(h)),3)
    assert len(bits)%8==0
    return packed(bits),selected

def sources():
    core=eld(True,[(n,24000,c,'base',0,0,False)for n in [480,512]for c in [1,2]])
    ldroot=SOURCE/'aac-eld-sbr';lm=json.loads((ldroot/'manifest.json').read_text());lp=(ldroot/'packets.bin').read_bytes()
    for ld in [False,True]:
        for base in json.loads(core['manifest.json'])['cases']:
            n=base['n'];channels=base['channels'];original=next(c for c in lm['cases']if c['name']==f'n{n}-c{channels}-x2-crc1')if ld else base
            src=lp if ld else core['packets.bin'];gold=(ldroot/(original['name']+'.f32le')).read_bytes()if ld else core[base['name']+'.f32le']
            setup=field(31,5)+field(7,6)+frequency(24000)+field(channels,4)+field(n==480,1)+'000'+field(ld,1)+('11'+sbr_header(0,True)if ld else '')+'0000'
            yield dict(name=f'eld-n{n}-c{channels}-ld{int(ld)}',aot=39,n=n,channels=channels,rate=48000 if ld else 24000,setup=setup+'10',plain_asc=packed(setup+'00').hex(),original=original,blob=src,gold=gold)
    for stem in ['aac-er-lc','aac-er-ltp','aac-ld-filterbank']:
        m=json.loads((SOURCE/(stem+'.json')).read_text());src=(SOURCE/(stem+'-packets.bin')).read_bytes();gold=(SOURCE/(stem+'-reference.f32le')).read_bytes()
        for n in ([480,512]if stem=='aac-ld-filterbank'else [960,1024]):
            original=next(c for c in m['cases']if c['n']==n and c['channels']==1);aot,setup=er_prefix(bytes.fromhex(original['asc']));a=original['reference_offset']
            yield dict(name=stem+'-'+original['name'],aot=aot,n=n,channels=1,rate=24000,setup=setup,plain_asc=original['asc'],original=original,blob=src,gold=gold[a:a+original['reference_bytes']])

def generate():
    files={};cases=[];bad=[];invalid=[];blob=bytearray();plain=bytearray();padding=[]
    for source in sources():
        original=source['original'];n=source['n'];channels=source['channels'];rate=source['rate'];source_n=n*(2 if rate==48000 else 1)
        raw=[source['blob'][r['offset']:r['offset']+r['bytes']]for r in original['frames']];width=max(map(len,raw))+2
        units=[r+bytes(width-len(r))for r in raw]
        padding.append(dict(name=source['name'],asc=source['plain_asc'],raw=[r.hex()for r in units],golden=source['name']+'-padding.f32le'));files[source['name']+'-padding.f32le']=source['gold']
        for frames in range(2,8):
            count=len(units)//frames;used=count*frames;golden=source['name']+f'-f{frames}.f32le';files[golden]=source['gold'][:used*source_n*channels*4]
            profiles=[('srcpc',0),('srcpc',1),('srcpc',2),('mixed',2),('rs',0)]
            if source['name']=='eld-n512-c1-ld0'and frames==7:profiles.append(('wide',1))
            for family,kind in profiles:
                lengths=([1]*62+[width*8-62])if family=='wide'else[3,0,5,width*8-8]if family=='srcpc'else[8,0,8,width*8-16]
                for flags in ['joined','separate','mixed']:
                    name=source['name']+f'-f{frames}-{family}-i{kind}-{flags}';sets,ep=config(lengths,frames,family,kind,flags);asc=packed(source['setup']+ep);files[name+'.asc']=asc
                    variants=['clean']
                    if channels==1 and family=='srcpc'and kind==0 and flags=='mixed':
                        variants +=['crc','truncated']
                        if source['aot']==39 and '-ld0'in source['name']:variants +=['aac-alignment']
                    for mutation in variants:
                        label=name if mutation=='clean'else name+'-'+mutation;rows=[];packets=[];bad_index=min(3,count-1)
                        for i in range(count):
                            children=list(units[i*frames:(i+1)*frames])
                            if mutation=='aac-alignment'and i==bad_index:
                                assert len(original['frames'][(i+1)*frames-1]['bits'])%8
                                changed=bytearray(children[-1]);changed[-1]|=1;children[-1]=bytes(changed)
                            packet,selected=protect(children,sets,kind,i,mutation if i==bad_index and mutation in ['crc','truncated']else None)
                            offsets=[]
                            for unit in children:offsets.append(dict(offset=len(plain),bytes=len(unit)));plain.extend(unit)
                            rows.append(dict(offset=len(blob),bytes=len(packet),choice=selected,units=offsets));blob.extend(packet);packets.append(packet)
                        c=dict(name=label,n=n,aot=source['aot'],channels=channels,rate=rate,source_frames=frames,frame_samples=source_n,container_rate=rate,container_frame_samples=source_n*frames,samples=used*source_n,pcm_offset=0,slots=n//64,bands=64 if rate==48000 else 32,asc=asc.hex(),plain_asc=source['plain_asc'],frames=rows,golden=golden,configs=sets)
                        c['video']=label+'.mp4';files[c['video']]=video_fixture([c],blob,channels,filename=c['video'],write=False)['data'];c['matroska']=label+'.mka';files[c['matroska']]=mkv(asc,packets,rate,source_n*frames,channels)
                        if mutation=='clean':cases.append(c)
                        else:c.update(good=name,bad_index=bad_index,error='EP class CRC mismatch'if mutation=='crc'else'nonzero AAC ELD alignment bits'if mutation=='aac-alignment'else'EP class/packet length mismatch');bad.append(c)
                        if mutation=='clean'and source['name']=='eld-n512-c1-ld0'and frames==2 and family=='srcpc'and kind==0 and flags=='mixed':
                            for escape in ['length','rate','crc']:
                                _,invalid_ep=config(lengths,frames,family,kind,flags,escape)
                                refused=dict(c,name=name+'-escaped-'+escape,asc=packed(source['setup']+invalid_ep).hex(),error='EP concatenated frames forbid escaped class parameters')
                                refused['video']=refused['name']+'.mp4';files[refused['video']]=video_fixture([refused],blob,channels,filename=refused['video'],write=False)['data'];refused['matroska']=refused['name']+'.mka';files[refused['matroska']]=mkv(bytes.fromhex(refused['asc']),packets,rate,source_n*frames,channels);invalid.append(refused)
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain);files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,invalid_configs=invalid,padding=padding,provenance='Own saved ELD/LD-SBR and ER LC/LTP/LD PCM, valid zero EXT_FILL, fixed source-class partitions and independent polynomial/matrix protection. No private media, foreign decoder, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
