"""Own EP3 extension categories 5..8, explicit offline generation only."""
import copy,json
from pathlib import Path
from generate_aac_er_ep_fixtures import prefix
from generate_aac_er_lc_esc_fixtures import trace as lc_trace
from generate_aac_er_predictive_esc_fixtures import trace as predictive_trace
from generate_aac_eld_esc_fixtures import configuration,protect
from generate_aac_eld_fixtures import mkv
from generate_aac_sbr_dsp_fixtures import header
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-esc-extensions'

def sources():
    for stem,names in [('aac-er-lc',['960-1-0-0','1024-6-2-1']),('aac-er-ltp-resilient',['960-7-mono','1024-7-common-2']),('aac-ld-ltp',['480-0']),('aac-ld-stereo',['512-7-common-2'])]:
        m=json.loads((SOURCE/(stem+'.json')).read_text());blob=(SOURCE/(stem+'-packets.bin')).read_bytes();gold=(SOURCE/(stem+'-reference.f32le')).read_bytes()
        for base in m['cases']:
            if base['name']not in names:continue
            aot,setup=prefix(bytes.fromhex(base['asc']));frames=[]
            for index,row in enumerate(base['frames']):
                wire,parts=lc_trace(base,index,False)if aot==17 else predictive_trace(stem,base,index)
                assert packed(wire)==blob[row['offset']:row['offset']+row['bytes']]
                pad=(-len(wire))%8
                if pad:parts[-1]['bits']=parts[-1]['bits'][:-pad]
                assert sum(len(p['bits'])for p in parts)==len(wire)
                frames.append(dict(wire=wire,esc=parts))
            offset=base.get('reference_offset',base['frames'][0].get('reference_offset',0));size=len(frames)*base['n']*base['channels']*4
            yield dict(name=stem+'-'+base['name'],aot=aot,n=base['n'],channels=base['channels'],rate=24000,configuration=base['channels'],flags=base.get('flags',0),setup=setup[:-2],plain_asc=base['asc'],frames=frames,golden=gold[offset:offset+size])
    path=SOURCE/'aac-eld-esc-sbr';m=json.loads((path/'manifest.json').read_text());raw=(path/'plain-packets.bin').read_bytes();info=(path/'instances.bin').read_bytes()
    for name,groups in [('n480-c1-x2-crc1',1),('n512-c7-x2-crc1',4)]:
        base=next(c for c in m['cases']if c['name']==name);frames=[]
        for row in base['frames']:
            parts=[dict(category=p['category'],bits=''.join(str((info[p['offset']+i//8]>>(7-i%8))&1)for i in range(p['bits'])))for p in row['instances']]
            count=sum(len(p['bits'])for p in parts);packet=raw[row['plain_offset']:row['plain_offset']+row['plain_bytes']];bits=''.join(f'{b:08b}'for b in packet)
            assert set(bits[count:])<=set('0')and len(bits)-count<8
            frames.append(dict(wire=bits[:count],esc=parts))
        setup=field(31,5)+field(7,6)+frequency(24000)+field(base['configuration'],4)+field(base['n']==480,1)+'000'+'111'+header(0,True)*groups+'0000'
        assert packed(setup+'00').hex()==base['plain_asc']
        yield dict(name='eld-'+name,aot=39,n=base['n'],channels=base['channels'],configuration=base['configuration'],rate=48000,flags=0,setup=setup,plain_asc=base['plain_asc'],frames=frames,golden=(path/(name+'.f32le')).read_bytes())

def ancillary(index,count):
    length='';left=count
    while left>=255:length+=field(255,8);left-=255
    length+=field(left,8)
    data=''.join(field((i*37+index)%256,8)for i in range(count))
    return [dict(category=5,bits='00100000'),dict(category=8,bits=length+data)]

def dynamic(base,index,full):
    if not full:return [dict(category=5,bits='1011'),dict(category=6,bits='0000'+field((index%2)*128+index,8))]
    groups=(base['channels']+6)//7;body='1'+'0000'+'0000'+'1'
    for group in range(groups):
        body+=''.join(field((group*7+ch)<base['channels']and(group*7+ch+index)%2==0,1)for ch in range(7))+field(group+1<groups,1)
    body+='1'+field(15,4)+'0000'+''.join(field((base['n']//4)*(i+1)//16-1,8)for i in range(16))
    body+='1'+field(90,7)+'0'+''.join(field((i%2)*128+(index+i)%32,8)for i in range(16))
    return [dict(category=5,bits='1011'),dict(category=6,bits=body)]

def extensions(base,index,kind):
    if kind=='fill-zero':return [dict(category=5,bits='0000'),dict(category=7,bits='110101001101')]
    if kind=='fill-data':return [dict(category=5,bits='0001'),dict(category=7,bits='0000'+'10100101'*(3+index%3))]
    if kind.startswith('anc-')and kind!='anc-version':return ancillary(index,{'anc-zero':0,'anc-short':7,'anc-escaped':255 if index%2 else 510}[kind])
    if kind=='anc-version':return [dict(category=5,bits='00100001'),dict(category=7,bits=field(index,8)+'1100101001010011')]
    if kind=='reserved':return [dict(category=5,bits=field([3,4,5,6,7,8,9,10,15][index%9],4)),dict(category=7,bits='1001'+'10110110'*2)]
    if kind=='drc-simple':return dynamic(base,index,False)
    assert kind=='chain';parts=dynamic(base,index,True)+ancillary(index,7)
    count=sum(len(p['bits'])for p in parts);parts+=[dict(category=5,bits='0000'),dict(category=7,bits='1'*((-(count+4))%8+8))]
    return parts

def generate():
    files={};cases=[];bad=[];blob=bytearray();plain=bytearray();info=bytearray()
    kinds=['fill-zero','fill-data','anc-zero','anc-short','anc-escaped','anc-version','drc-simple','chain','reserved']
    for base in sources():
        golden=base['name']+'.f32le';files[golden]=base['golden'];assert len(base['golden'])==len(base['frames'])*base['n']*(2 if base['rate']==48000 else 1)*base['channels']*4
        for kind in kinds:
            originals=[]
            for index,row in enumerate(base['frames']):
                ext=extensions(base,index,kind);assert sum(len(p['bits'])for p in ext)%8==0
                originals.append(dict(wire=row['wire'],esc=copy.deepcopy(row['esc'])+ext,core_classes=len(row['esc'])))
            patterns=list(dict.fromkeys(tuple(p['category']for p in row['esc'])for row in originals));orders=[list(range(len(p)))[::-1]if i%2 else list(range(len(p)))for i,p in enumerate(patterns)]
            _,_,_,ep=configuration(dict(rate=24000,configuration=1,n=512,flags=0),patterns,orders);asc=packed(base['setup']+'11'+ep+'1');name=base['name']+'-'+kind;files[name+'.asc']=asc
            variants=['clean']+({'fill-data':['fill-nibble','fill-byte'],'anc-short':['anc-length'],'chain':['short-type','short-drc','opaque-spill','anc-spill','alignment']}.get(kind,[]))
            for variant in variants:
                label=name if variant=='clean'else name+'-'+variant;rows=[];packets=[];bad_index=3
                if variant=='alignment':bad_index=next(i for i,r in enumerate(originals)if i>=3 and len(r['wire'])%8)
                for index,original in enumerate(originals):
                    parts=copy.deepcopy(original['esc']);ext=parts[original['core_classes']:]
                    if variant!='clean'and index==bad_index:
                        if variant=='fill-nibble':ext[1]['bits']='0001'+ext[1]['bits'][4:]
                        if variant=='fill-byte':ext[1]['bits']=ext[1]['bits'][:4]+'01011010'+ext[1]['bits'][12:]
                        if variant=='anc-length':ext[1]['bits']=field(8,8)+ext[1]['bits'][8:]
                        if variant=='short-type':ext[0]['bits']=ext[0]['bits'][:3]
                        if variant=='short-drc':ext[1]['bits']=ext[1]['bits'][:1]
                        if variant=='opaque-spill':ext[0]['bits']='00100001'
                        if variant=='anc-spill':ext[3]['bits']=field(8,8)+ext[3]['bits'][8:]
                        # Keep the enclosing ER extension area whole-byte sized;
                        # the target bounded class, not transport geometry, fails.
                        ext[-1]['bits']+='0'*(-sum(len(p['bits'])for p in ext)%8)
                    # Ext is a view of the same dictionaries as parts.
                    tail=''.join(p['bits']for p in ext);logical=original['wire']+tail;pad=(-len(logical))%8
                    parts[-1]['bits']+='0'*pad
                    if variant=='alignment'and index==bad_index:parts[-1]['bits']=parts[-1]['bits'][:-1]+'1'
                    packet,choice=protect(dict(esc=parts),patterns,orders)
                    locations=[]
                    for p in parts:
                        data=packed(p['bits']);locations.append(dict(category=p['category'],offset=len(info),bits=len(p['bits']),bytes=len(data)));info.extend(data)
                    raw=packed(logical);rows.append(dict(offset=len(blob),bytes=len(packet),choice=choice,instances=locations,plain_offset=len(plain),plain_bytes=len(raw),core_classes=original['core_classes']));blob.extend(packet);plain.extend(raw);packets.append(packet)
                c=dict(name=label,kind=kind,aot=base['aot'],n=base['n'],channels=base['channels'],configuration=base['configuration'],rate=base['rate'],flags=base['flags'],asc=asc.hex(),plain_asc=base['plain_asc'],ep2_asc=packed(base['setup']+'10'+ep).hex(),reserved_asc=packed(base['setup']+'11'+ep+'0').hex(),frames=rows,golden=golden,container_rate=base['rate'],container_frame_samples=base['n']*(2 if base['rate']==48000 else 1),samples=len(rows)*base['n']*(2 if base['rate']==48000 else 1),pcm_offset=0,slots=base['n']//64,bands=64 if base['rate']==48000 else 32,video=label+'.mp4',matroska=label+'.mka')
                files[c['video']]=video_fixture([c],blob,base['channels'],filename=c['video'],write=False)['data'];files[c['matroska']]=mkv(asc,packets,base['rate'],c['container_frame_samples'],base['channels'])
                if variant=='clean':cases.append(c)
                else:
                    error={'fill-nibble':'invalid AAC fill-data nibble','fill-byte':'invalid AAC fill-data byte','anc-length':'AAC ancillary data exceeds fill payload','short-type':'truncated or oversized bit field','short-drc':'truncated or oversized bit field','opaque-spill':'AAC ESC extension exceeds category boundary','anc-spill':'AAC ESC extension exceeds category boundary','alignment':'nonzero AAC ESC alignment bits'}[variant]
                    c.update(good=name,bad_index=bad_index,error=error,variant=variant);bad.append(c)
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain);files['instances.bin']=bytes(info)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,provenance='Own core source bits and independent numerical PCM; explicit synthetic category5..8 metadata, DRC syntax without optional gain application, ancillary length escapes and opaque/fill payloads. No private media, foreign codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
