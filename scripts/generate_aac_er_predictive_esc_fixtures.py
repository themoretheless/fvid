"""Own ER-LTP/LD EP3 categories, predictors and resilience, explicit/offline."""
import copy,json
from pathlib import Path
from generate_aac_main_tools_fixtures import channel as ordinary,ics
from generate_aac_er_ltp_resilient_fixtures import channel as resilient,prediction
from generate_aac_ld_stereo_fixtures import predictor as ld_prediction
from generate_aac_er_ep_fixtures import prefix
from generate_aac_eld_esc_fixtures import configuration,protect
from generate_aac_eld_fixtures import mkv
from generate_he_aac_packet_fixtures import field,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-er-predictive-esc'

def body_parts(seq,values,info,kwargs=None,er_tns=None):
    if kwargs is not None:
        books=kwargs.pop('books')
        parts=resilient(seq,books,values,info=info,categories=True,**kwargs)
        assert ''.join(p['bits']for p in parts)==resilient(seq,books,values,info=info,**kwargs)
        return parts
    body=ordinary(seq,[1,1],values,info=info,tns=er_tns,er=er_tns is not None)
    side=8+len(info)+(14 if seq==2 else 18)+2+3
    parts=[dict(category=1,bits=body[:side])]
    if er_tns is not None:parts.append(dict(category=3,bits=body[side:side+19]));side+=19
    parts.append(dict(category=4,bits=body[side:]));return parts

def trace(stem,c,index):
    n=c['n'];mode=c.get('mode',c['name'].replace('-extension','').split('-',2 if 'flags'in c else 1)[-1]);flags=c.get('flags',0);width=c['channels'];common=mode.startswith('common');ms=int(mode[-1])if common else 0
    parts=[];wire='';row=c['frames'][index];predictors=[];infos=[];values=[];kwargs=[]
    if stem.startswith('aac-er-ltp'):
        seq=row['sequence'];header=ics(seq,2,False,shape=index%2);active=[seq!=2 and index>=1 and(index+ch)%4!=0 for ch in range(width)]
        for ch in range(width):
            pred=prediction(active[ch],n-13*((index+ch)%3),(index+3*ch)%8,[True,(index+ch)%3!=1]);predictors.append(pred)
            infos.append(header if seq==2 else header[:-1]+field(active[ch],1)+(pred if active[ch]else ''))
            q=[[(-1 if(index+w+k+ch)%3==0 else 1 if(index+w+k+ch)%3==1 else 0)for k in range(8)]for w in range(8 if seq==2 else 1)]
            kw=None
            if stem.endswith('resilient'):
                books=[17 if flags&4 else 1,1]
                if index==0:books=[0,0];q=[[0]*8]
                if books[0]==17:
                    for r in q:r[:4]=[v*17 for v in r[:4]]
                delta=[0,-8,8,7,-7,6,-6,1,-1,0,8,-8][index]
                kw=dict(books=books,flags=flags,scale_values=[140+delta,140],tns=bool((index+ch)%2)if seq!=2 else None)
            kwargs.append(kw);values.append(q)
        present=seq!=2 and any(active);shared=header if seq==2 else header[:-1]+field(present,1)
    elif stem=='aac-ld-stereo':
        seq=0;active=[index>=1 and(index+ch)%4!=0 for ch in range(2)];present=any(active)
        for ch in range(2):
            shape=(index//2)%2 if common or ch==0 else ((index+1)//3)%2
            update=(1023 if(index+ch)%3==0 else 13+71*ch)if active[ch]and index in(1,3,6,9)else None
            pred=ld_prediction(active[ch],update,(index+3*ch)%8,[True,(index+ch)%3!=1]);predictors.append(pred)
            header='000'+field(shape,1)+field(2,6);infos.append(header+field(active[ch],1)+(pred if active[ch]else ''))
            q=[(index+k+ch)%3-1 for k in range(8)]
            if flags&4:q[:4]=[v*17 for v in q[:4]]
            values.append([q]);kwargs.append(dict(books=[17 if flags&4 else 1,1],flags=flags,scale_values=[140+(-8,7,0,8)[(index+ch)%4],140],tns=bool((index+ch)%2)))
        shared=infos[0][:10]+field(present,1)
    else:
        seq=0;common=False;present=False;active=[bool(row.get('active',False))];kwargs=[None]
        q=[int(v/1024)for v in row['spectrum']];values=[[q]];pred=''
        header='000'+field(row['shape'],1)+field(2,6)+field(active[0],1)
        if active[0]:pred=ld_prediction(True,row['lag_update'],row['coefficient'],row['used'])
        infos=[header+pred];predictors=[pred];shared=''
    if width==2:
        mask=(''.join(field(v,1)for v in [index%2==0,index%3==0])if stem=='aac-ld-stereo'else '10')if ms==1 else ''
        text='0000'+field(common,1)+(shared+field(ms,2)+mask if common else '')
        parts.append(dict(category=0,bits=text));wire+=text
    for ch in range(width):
        info=''if common else infos[ch]
        pred=predictors[ch]if common and present else ''
        er_tns=(bool(c['reverse']),1)if stem=='aac-ld-ltp'else None
        body=body_parts(seq,values[ch],info,copy.deepcopy(kwargs[ch]),er_tns)
        text=''.join(p['bits']for p in body);tag='0000'if width==1 else ''
        wire+=tag+pred+text;body[0]['bits']=tag+pred+body[0]['bits']
        if active[ch]:
            # ltp_data_present is channel-dependent, before global_gain in a
            # common-window pair, and inside the independent ICS otherwise.
            at=0 if common else len(tag)+8+(15 if seq==2 else 11)
            body[0]['prediction_start']=at
            if stem.startswith('aac-er-ltp'):body[0]['lag_start']=at+1
        parts+=body
    parts[-1]['bits']+='0'*(-len(wire)%8)
    return wire,parts

def generate():
    files={};cases=[];bad=[];blob=bytearray();plain=bytearray();instances=bytearray()
    for stem in ['aac-er-ltp','aac-er-ltp-resilient','aac-ld-ltp','aac-ld-filterbank','aac-ld-stereo']:
        source=json.loads((SOURCE/(stem+'.json')).read_text());raw=(SOURCE/(stem+'-packets.bin')).read_bytes();gold=(SOURCE/(stem+'-reference.f32le')).read_bytes()
        for base in source['cases']:
            if 'large_channel'in base:continue
            name=stem+'-'+base['name'];aot,setup=prefix(bytes.fromhex(base['asc']));setup=setup[:-2]
            frames=[]
            for i,row in enumerate(base['frames']):
                wire,parts=trace(stem,base,i);assert packed(wire)==raw[row['offset']:row['offset']+row['bytes']],(name,i)
                frames.append(dict(wire=wire,esc=parts))
            patterns=list(dict.fromkeys(tuple(p['category']for p in row['esc'])for row in frames));orders=[list(range(len(p)))[::-1]if i%2 else list(range(len(p)))for i,p in enumerate(patterns)]
            _,_,_,ep=configuration(dict(rate=24000,configuration=1,n=512,flags=0),patterns,orders)
            asc=packed(setup+'11'+ep+'1');files[name+'.asc']=asc
            start=base.get('reference_offset',base['frames'][0].get('reference_offset',0));size=len(frames)*base['n']*base['channels']*4;files[name+'.f32le']=gold[start:start+size];assert len(files[name+'.f32le'])==size
            mode=base.get('mode',base['name'].replace('-extension','').split('-',2 if 'flags'in base else 1)[-1]);flags=base.get('flags',0)
            selected=(stem in ['aac-ld-ltp','aac-ld-filterbank'])or(mode in ['mono','common-2','independent']and flags in [0,7]and not base['name'].endswith('-extension'))
            variants=['clean']+(['short-side','short-spectrum','crc','alignment']if selected else [])
            if selected and aot==19 and base['n']==960:variants.append('lag-overflow')
            for variant in variants:
                label=name if variant=='clean'else name+'-'+variant;rows=[];packets=[];bad_index=3
                if variant in ['short-side','lag-overflow'] and any('prediction_start'in p for f in frames[3:]for p in f['esc']):bad_index=next(i for i,f in enumerate(frames)if i>=3 and any('prediction_start'in p for p in f['esc']))
                if variant=='alignment':bad_index=next(i for i,f in enumerate(frames)if i>=3 and len(f['wire'])%8)
                for i,original in enumerate(frames):
                    row=copy.deepcopy(original)
                    if i==bad_index:
                        if variant=='short-side':
                            p=next((p for p in row['esc']if 'prediction_start'in p),next(p for p in row['esc']if p['category']==1));p['bits']=p['bits'][:p.get('prediction_start',-1)+1]
                        if variant=='short-spectrum':next(p for p in reversed(row['esc'])if p['category']==4)['bits']=''
                        if variant=='alignment':p=row['esc'][-1];p['bits']=p['bits'][:-1]+'1'
                        if variant=='lag-overflow':
                            assert any('lag_start'in p for p in row['esc']),(name,i)
                            p=next(p for p in row['esc']if 'lag_start'in p);at=p['lag_start'];p['bits']=p['bits'][:at]+field(1921,11)+p['bits'][at+11:]
                    packet,choice=protect(row,patterns,orders,'crc'if variant=='crc'and i==bad_index else None)
                    locations=[]
                    for item in row['esc']:
                        data=packed(item['bits']);locations.append(dict(category=item['category'],offset=len(instances),bytes=len(data),bits=len(item['bits'])));instances.extend(data)
                    logical=packed(original['wire']);rows.append(dict(offset=len(blob),bytes=len(packet),choice=choice,instances=locations,plain_offset=len(plain),plain_bytes=len(logical)));blob.extend(packet);plain.extend(logical);packets.append(packet)
                c=dict(name=label,aot=aot,n=base['n'],channels=base['channels'],rate=24000,flags=flags,mode=mode,asc=asc.hex(),plain_asc=base['asc'],ep2_asc=packed(setup+'10'+ep).hex(),reserved_asc=packed(setup+'11'+ep+'0').hex(),frames=rows,golden=name+'.f32le',container_rate=24000,container_frame_samples=base['n'],samples=len(frames)*base['n'],pcm_offset=0,slots=base['n']//64,bands=32,video=label+'.mp4',matroska=label+'.mka')
                files[c['video']]=video_fixture([c],blob,base['channels'],filename=c['video'],write=False)['data'];files[c['matroska']]=mkv(asc,packets,24000,base['n'],base['channels'])
                if variant=='clean':cases.append(c)
                else:c.update(good=name,bad_index=bad_index,error='AAC LTP lag exceeds two frames'if variant=='lag-overflow'else'EP class CRC mismatch'if variant=='crc'else'nonzero AAC ESC alignment bits'if variant=='alignment'else'truncated AAC HCR spectral region'if variant=='short-spectrum'and flags&1 else'truncated or oversized bit field');bad.append(c)
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain);files['instances.bin']=bytes(instances)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,provenance='Own saved ER-LTP/LD source bits, independent category tracing and polynomial EP protection, independent scalar prediction/filterbank/TNS numerical references. No private media, foreign codec, process or network. Large independent HCR regions remain separately qualified.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
