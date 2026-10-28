"""Own EP3 standard ER SBR category 9; offline source tracing and QMF PCM."""
import copy,json,math,struct
from pathlib import Path
from generate_aac_er_multi_sbr_fixtures import LAYOUTS,sbr
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_eld_esc_fixtures import configuration,protect
from generate_aac_eld_fixtures import mkv
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-esc-sbr'
def bits(raw):return ''.join(f'{b:08b}'for b in raw)
def asc(c,ep=None):
    ga=field(c['n']==960,1)+'00'+('00'if ep is None else'11'+ep+'1')
    if c.get('signal')in ['sync','implicit']:return packed(field(c['aot'],5)+frequency(24000)+field(c['configuration'],4)+ga+field(0x2b7,11)+field(5,5)+'1'+frequency(c['rate'])+(field(0x548,11)+'1'if c.get('ps')and c['signal']=='sync'else''))
    return packed(field(29 if c.get('ps')else 5,5)+frequency(24000)+field(c['configuration'],4)+frequency(c['rate'])+field(c['aot'],5)+ga)
def sources():
    scalar=(SOURCE/'aac-sbr-dsp-pcm.f64le').read_bytes()
    mono=json.loads((SOURCE/'aac-er-sbr.json').read_text());blob=(SOURCE/'aac-er-sbr-packets.bin').read_bytes()
    core='0000'+field(100,8)+'0000'+'000000'+'0'+'000'
    for c in mono['cases']:
        frames=[]
        for r in c['frames']:
            raw=bits(blob[r['offset']:r['offset']+r['bytes']]);ext=raw[len(core):len(raw)-(-len(core)%8)]
            assert len(ext)%8==0 and ext[:4]=='1101'
            frames.append([dict(category=1,bits=core),dict(category=4,bits=''),dict(category=5,bits=ext[:4]),dict(category=9,bits=ext[4:])])
        golden=b''.join(struct.pack('<f',struct.unpack_from('<d',scalar,c['pcm_offset']+8*i)[0])for i in range(c['samples']))
        yield dict(name='mono-'+c['name'],aot=c['aot'],n=c['slots']*64,channels=1,configuration=1,rate=c['container_rate'],signal='sync'if c['name'].endswith('sync')else'explicit',frames=frames,golden=golden)
    m=json.loads((SOURCE/'aac-er-multi-sbr.json').read_text());original=(SOURCE/'aac-er-multi-sbr-packets.bin').read_bytes();_,hi,_,_=tables(10,27,0,False,0,0)
    channel=field(100,8)+'0000'+'000000'+'0'+'000'
    for c in m['cases']:
        kinds,mapping,_=LAYOUTS[c['layout']];frames=[]
        for index,row in enumerate(c['frames']):
            parts=[];extensions=[];offset=0
            for tag,kind in enumerate(kinds,1):
                width=2 if kind==1 else 1
                if kind==1:parts.append(dict(category=0,bits=field(tag,4)+'0'))
                for ch in range(width):parts.extend([dict(category=1,bits=(''if kind==1 else field(tag,4))+channel),dict(category=4,bits='')])
                if kind!=3:
                    ext=bits(sbr(list(range(offset,offset+width)),index,len(hi)-1));extensions.extend([dict(category=5,bits=ext[:4]),dict(category=9,bits=ext[4:])])
                offset+=width
            parts+=extensions;wire=''.join(p['bits']for p in parts);assert packed(wire)==original[row['offset']:row['offset']+row['bytes']];frames.append(parts)
        golden=b''
        for i in range(c['samples']):
            value=struct.unpack_from('<d',scalar,c['pcm_offset']+i*8)[0];pcm=[0.]*c['channels']
            for coded,output in enumerate(mapping):pcm[output]=value*c['scales'][coded]
            golden+=struct.pack('<'+'f'*len(pcm),*pcm)
        yield dict(name='multi-'+c['name'],aot=c['aot'],n=c['slots']*64,channels=c['channels'],configuration=c['layout'],rate=c['container_rate'],frames=frames,golden=golden)
def ps_sources():
    from generate_aac_ep_ps_fixtures import originals
    raw=originals(frames=6,include_bits=True)
    for c in json.loads(raw['aac-er-ps.json'])['cases']:
        frames=[]
        for row in c['frames']:
            side=46+(17 if row['active']else 0);core=row['core_bits'];extension=bits(bytes.fromhex(row['payload']))
            frames.append([dict(category=1,bits=core[:side]),dict(category=4,bits=core[side:]),dict(category=5,bits=extension[:4]),dict(category=9,bits=extension[4:])])
        yield dict(name='ps-'+c['name'],ps=True,aot=c['aot'],n=c['frame_samples'],channels=2,configuration=1,rate=c['rate'],signal=c['signal'],frames=frames,core_reference=c['core_reference'],core_pcm=raw[c['core_reference']],payloads=[r['payload']for r in c['frames']])
def multi_ps_sources(ps_enabled=True):
    from generate_aac_ep_ps_fixtures import originals
    from generate_aac_ps_fixtures import ps,sized,sbr_word,sbr_header
    from generate_aac_ep_concatenated_fixtures import config as ep_config,protect as ep_protect
    raw=originals(frames=6,include_bits=True);nh=len(tables(10,27,0,False,0,0)[1])-1
    for c in json.loads(raw['aac-er-ps.json'])['cases']:
        if c['signal']!='implicit' or c['rate']!=48000:continue
        units=[];payloads=[];parts=[];core=c['frames'][0]['core_bits']
        for i in range(14):
            text,_=ps(1,1,i%3,slots=c['frame_samples']//32,include_header=i%3==0)
            temporal=i>0;env=sbr_word(0,0)*nh if temporal else field(2,7)+sbr_word(1,0)*(nh-1);noise=sbr_word(8,0)if temporal else field(7,5)
            prefix='1101'+field(i%4==0,1)+(sbr_header(2,True)if i%4==0 else'')+'0'+'00001'+field(temporal,1)*2+'00'+env+noise+'0'+'1'
            count=160-(len(prefix)+12+7)//8;inner='10'+text if ps_enabled else'00';assert len(inner)+2<count*8
            # A reserved SBR id terminates the PS extension list; remaining
            # bytes belong to that bounded opaque extension, not AAC padding.
            area=inner+'00'+'1'*(count*8-len(inner)-2)
            payload=packed(prefix+'1111'+field(count-15,8)+area);assert len(payload)==160
            wire=core+bits(payload);unit=packed(wire);p=[core[:46],core[46:],'1101',bits(payload)[4:]+'0'*(-len(wire)%8)]
            units.append(unit);payloads.append(payload.hex());parts.append(p)
        for frames in [2,3,7]:
            lengths=list(map(len,parts[0]));assert all(list(map(len,p))==lengths for p in parts)
            for family,kind,flagmode in [('srcpc',0,'mixed'),('srcpc',1,'separate')]:
                sets,ep=ep_config(lengths,frames,family,kind,flagmode);base=dict(name=('multi-ps-'if ps_enabled else'multi-sbr-')+c['name']+f'-f{frames}-i{kind}',ps=ps_enabled,multi=True,aot=c['aot'],n=c['frame_samples'],channels=2 if ps_enabled else 1,configuration=1,rate=c['rate'],signal=c['signal']if ps_enabled else'sync',source_frames=frames,sets=sets,interleave=kind,payloads=payloads[:len(units)//frames*frames],units=units[:len(units)//frames*frames],ep=ep)
                yield base
def generate():
    files={};cases=[];ps_cases=[];bad=[];blob=bytearray();plain=bytearray();info=bytearray()
    from itertools import chain
    for base in chain(sources(),ps_sources()):
        name=base['name'];frames=base['frames'];patterns=list(dict.fromkeys(tuple(p['category']for p in row)for row in frames));orders=[list(range(len(p)))[::-1]if i%2 else list(range(len(p)))for i,p in enumerate(patterns)]
        _,_,_,ep=configuration(dict(rate=24000,configuration=1,n=512,flags=0),patterns,orders)
        setup=asc(base,ep);plain_setup=asc(base);files[name+'.asc']=setup
        if 'golden'in base:files[name+'.f32le']=base['golden']
        if base.get('ps'):files[base['core_reference']]=base['core_pcm']
        variants=['clean']+([]if base.get('ps')else ['short-sbr','spill','missing-header','crc']if base['channels']==1 else ['missing','excess','order']if base['configuration']==14 else [])
        for variant in variants:
            label=name if variant=='clean'else name+'-'+variant;rows=[];packets=[];bad_index=0 if variant=='missing-header'else 1
            for index,original in enumerate(frames):
                parts=copy.deepcopy(original)
                if variant!='clean'and index==bad_index:
                    at=next(i for i,p in enumerate(parts)if p['category']==9)
                    if variant=='short-sbr':parts[at]['bits']=parts[at]['bits'][:4]
                    if variant=='spill':parts[at]['bits']+='10101010'
                    if variant=='missing-header':parts[at]['bits']='0'+parts[at]['bits'][1:]
                    if variant=='crc':parts[at-1]['bits']='1110'
                    if variant=='missing':parts=parts[:-2]
                    if variant=='excess':parts+=copy.deepcopy(parts[-2:])
                    if variant=='order':parts+=[dict(category=5,bits='00100000'),dict(category=8,bits='00000000')]
                logical=''.join(p['bits']for p in parts);pad=(-len(logical))%8;parts[-1]['bits']+='0'*pad
                pattern=tuple(p['category']for p in parts)
                if pattern not in patterns:patterns.append(pattern);orders.append(list(range(len(pattern))))
                # Store sources first; EP setup is finalized after malformed topology adds sets.
                instances=[]
                for part in parts:
                    raw=packed(part['bits']);instances.append(dict(category=part['category'],offset=len(info),bytes=len(raw),bits=len(part['bits'])));info.extend(raw)
                raw=packed(logical);rows.append(dict(parts=parts,instances=instances,plain_offset=len(plain),plain_bytes=len(raw)));plain.extend(raw)
            _,_,_,final_ep=configuration(dict(rate=24000,configuration=1,n=512,flags=0),patterns,orders)
            variant_asc=asc(base,final_ep)
            for index,row in enumerate(rows):
                original=copy.deepcopy(frames[index]);original[-1]['bits']+='0'*(-sum(len(p['bits'])for p in original)%8)
                recovery,_=protect(dict(esc=original),patterns,orders);row.update(good_offset=len(blob),good_bytes=len(recovery));blob.extend(recovery)
                packet,choice=protect(dict(esc=row.pop('parts')),patterns,orders);row.update(offset=len(blob),bytes=len(packet),choice=choice);blob.extend(packet);packets.append(packet)
            c={k:v for k,v in base.items()if k not in ['frames','golden','core_pcm']};c.update(name=label,asc=variant_asc.hex(),plain_asc=plain_setup.hex(),frames=rows,golden=name+'.f32le',video=label+'.mp4',matroska=label+'.mka',container_rate=base['rate'],container_frame_samples=base['n']*base['rate']//24000,samples=len(frames)*base['n']*base['rate']//24000,pcm_offset=0,slots=base['n']//64,bands=64 if base['rate']==48000 else 32)
            files[c['video']]=video_fixture([c],blob,base['channels'],filename=c['video'],write=False)['data'];files[c['matroska']]=mkv(variant_asc,packets,base['rate'],c['container_frame_samples'],base['channels'])
            if variant=='clean':(ps_cases if base.get('ps')else cases).append(c)
            else:c.update(good=name,bad_index=bad_index,variant=variant);bad.append(c)
    multiple=[];multi_sbr=[]
    from generate_aac_ep_concatenated_fixtures import protect as protect_multi
    for base in chain(multi_ps_sources(),multi_ps_sources(False)):
        name=base['name'];setup=asc(base,base['ep']);plain_setup=asc(base);rows=[];packets=[];count=base['source_frames'];units=base['units']
        for i in range(len(units)//count):
            packet,choice=protect_multi(units[i*count:(i+1)*count],base['sets'],base['interleave'],i);children=[]
            for unit in units[i*count:(i+1)*count]:children.append(dict(offset=len(plain),bytes=len(unit)));plain.extend(unit)
            children_raw=units[i*count:(i+1)*count].copy();bad_bits=bits(children_raw[-1]);children_raw[-1]=packed(bad_bits[:58]+'1110'+bad_bits[62:]);bad_packet,_=protect_multi(children_raw,base['sets'],base['interleave'],i)
            row=dict(bad_offset=len(blob),bad_bytes=len(bad_packet));blob.extend(bad_packet)
            row.update(offset=len(blob),bytes=len(packet),choice=choice,units=children);rows.append(row);blob.extend(packet);packets.append(packet)
        c={k:v for k,v in base.items()if k not in ['sets','ep','units']};c.update(asc=setup.hex(),plain_asc=plain_setup.hex(),frames=rows,container_rate=base['rate'],container_frame_samples=base['n']*2*count,samples=len(units)*base['n']*2,pcm_offset=0,slots=base['n']//64,bands=64,video=name+'.mp4',matroska=name+'.mka')
        files[c['video']]=video_fixture([c],blob,base['channels'],filename=c['video'],write=False)['data'];files[c['matroska']]=mkv(setup,packets,base['rate'],c['container_frame_samples'],base['channels']);(multiple if base['ps']else multi_sbr).append(c)
        damaged=[blob[r['bad_offset']:r['bad_offset']+r['bad_bytes']]if i==0 else packets[i]for i,r in enumerate(rows)]
        wrong=dict(c,frames=[dict(offset=r['bad_offset']if i==0 else r['offset'],bytes=r['bad_bytes']if i==0 else r['bytes'])for i,r in enumerate(rows)])
        c['bad_video']=name+'-late-sbr-crc.mp4';c['bad_matroska']=name+'-late-sbr-crc.mka'
        files[c['bad_video']]=video_fixture([wrong],blob,base['channels'],filename=c['bad_video'],write=False)['data'];files[c['bad_matroska']]=mkv(setup,damaged,base['rate'],c['container_frame_samples'],base['channels'])
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain);files['instances.bin']=bytes(info);files['manifest.json']=(json.dumps(dict(cases=cases,ps_cases=ps_cases,multiple=multiple,multi_sbr=multi_sbr,malformed=bad,provenance='Own tagged ER-LC/LTP sparse syntax, standard SBR and independent scalar QMF noise reference. No foreign codec/process/network/private media.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
