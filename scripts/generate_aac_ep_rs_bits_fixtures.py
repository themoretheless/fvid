"""Own RS data/CRC bit boundaries, including grouped non-octet members."""
import json
from pathlib import Path
from generate_aac_ep_rs_fixtures import encode,corrupt
from generate_aac_ep_playback_fixtures import crc,protected_header
from generate_aac_ep_class_codes_fixtures import octet_stage
from generate_aac_ep_multiclass_fixtures import interleave,window
from generate_aac_eld_fixtures import mkv
from generate_he_aac_packet_fixtures import field,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-ep-rs-bits'
def config(kind,switch,group,cut,reverse,until,k):
    order=[1,0]if reverse else[0,1];widths=[3,5]if group else[8-cut,cut]
    ep=field(1,8)+field(kind,2)+'001'+'001'+field(2,6)
    for index in range(2):
        fec=2 if group and index==order[0]else 1
        ep+='100'+field(fec,2)+(field(switch,2)if kind==2 else '')+'0'+field(0 if until and index==order[-1]else 15,4)+field(k,7)+field(widths[index],5)
    ep+='1'+''.join(field(i,6)for i in order)+'1'+field(24,5)+field(16,5)
    return ep,order,widths

def protect(raw,kind,switch,group,cut,reverse,until,k,mutation=None):
    _,order,widths=config(kind,switch,group,cut,reverse,until,k)
    bits=''.join(f'{v:08b}'for v in raw);parts=[bits[:cut],bits[cut:]]
    payloads=[p+crc(p,w)for p,w in zip(parts,widths)]
    if mutation=='bad-crc':
        index=order[0];payloads[index]=payloads[index][:-1]+str(1-int(payloads[index][-1]))
    data=[''.join(payloads[index]for index in order)]if group else[payloads[index]for index in order]
    assert all(len(p)%8==0 for p in data)
    coded=[]
    for p in data:
        encoded=encode(packed(p),k)
        if mutation=='corrected':encoded,_=corrupt(encoded,len(p)//8,k)
        coded.append(''.join(f'{v:08b}'for v in encoded))
    if mutation=='truncated':coded[-1]=coded[-1][:-8]
    if mutation=='empty-tail':coded=['']
    lengths=list(map(len,parts))
    if mutation=='bad-sum':lengths[order[0]]+=1
    attrs=''.join(field(lengths[i],15)for i in order if not(until and i==order[-1]))
    h=protected_header(attrs+'000',True);stuff=(-len(h)-sum(map(len,coded)))%8;h=protected_header(attrs+field(stuff,3),True)
    if kind==0 or kind==2 and switch in[0,3]:body=''.join(coded)
    else:
        body=''
        for c in reversed(coded):body=''.join(octet_stage(list(c),list(body)))
    wire=h+body+'0'*stuff if kind==0 else interleave(h,body+'0'*stuff,28)
    assert len(wire)%8==0
    return packed(wire),parts

def primitive_vectors():
    vectors=[]
    def descriptor(length,width,k,fec,until):
        code=17 if width==24 else 18 if width==32 else width
        return field(until,1)+'00'+field(fec,2)+'0'+(field(0,4)if until else field(length,16))+field(k,7)+field(code,5)
    def save(parts,widths,k,group,until):
        ep=field(1,8)+'00'+'000'+'001'+field(len(parts),6)
        for index,(bits,width)in enumerate(zip(parts,widths)):
            ep+=descriptor(len(bits),width,k,2 if group and index+1<len(parts)else 1,until and index+1==len(parts))
        ep+='00'
        message=''.join(bits+crc(bits,width)for bits,width in zip(parts,widths))
        assert len(message)%8==0
        encoded=encode(packed(message),k);damaged,corrected=corrupt(encoded,len(message)//8,k)
        vectors.append(dict(config=packed(ep).hex(),packet=encoded.hex(),damaged=damaged.hex(),corrected=corrected,k=k,widths=widths,parts=parts,until_end=until))
    for width in list(range(17))+[24,32]:
        for n in range(1,65):
            if (n+width)%8:continue
            for k in [0,2,127]:save([''.join(str((i*i+i//3+n)%2)for i in range(n))],[width],k,False,(n//8)%2==0)
        save(['10110',''.join(str((i*3+i//2)%2)for i in range(27))],[width,(-width)%8],2,True,width%2==0)
    return vectors

def ep3_cases(files,cases,blob,plain):
    path=SOURCE/'aac-eld-esc-sbr';m=json.loads((path/'manifest.json').read_text());info=(path/'instances.bin').read_bytes();raw=(path/'plain-packets.bin').read_bytes()
    from generate_he_aac_packet_fixtures import frequency
    from generate_aac_sbr_dsp_fixtures import header as sbr_header
    for name in ['n480-c1-x2-crc1','n512-c2-header-switch']:
        base=next(c for c in m['cases']if c['name']==name)
        rows=[]
        for frame in base['frames']:
            rows.append([''.join(str((info[p['offset']+i//8]>>(7-i%8))&1)for i in range(p['bits']))for p in frame['instances']])
        golden='ep3-'+name+'.f32le';files[golden]=(path/(name+'.f32le')).read_bytes()
        for group in [False,True]:
            for kind in [0,1]:
                parts=[r.copy()for r in rows]
                if group:
                    for r in parts:r[-1]+='0'*(-sum(map(len,r))%8)
                orders=[list(range(len(r)))[::-1]if i%2 else list(range(len(r)))for i,r in enumerate(parts)]
                widths=[([3,5]+[0]*(len(r)-2)if group else[(-len(p))%8 for p in r])for r in parts]
                ep=field(len(parts),8)+field(kind,2)+'001'+'001'
                for r,order,crc_widths in zip(parts,orders,widths):
                    ep+=field(len(r),6)
                    for index,(p,width)in enumerate(zip(r,crc_widths)):
                        fec=2 if group and index!=order[-1]else 1
                        ep+='000'+field(fec,2)+'0'+field(len(p),16)+field(2,7)+field(width,5)
                    ep+='1'+''.join(field(i,6)for i in order)
                ep+='1'+field(24,5)+field(16,5)
                prefix=field(31,5)+field(7,6)+frequency(24000)+field(base['configuration'],4)+field(base['n']==480,1)+'000'+'111'+sbr_header(0,True)+'0000'+'11'
                asc=packed(prefix+ep+'1')
                for corrected in [False,True]:
                    label='ep3-'+name+f'-ld1-g{int(group)}-i{kind}-'+('corrected'if corrected else'clean');frames=[];packets=[]
                    for index,(r,order,crc_widths)in enumerate(zip(parts,orders,widths)):
                        payloads=[p+crc(p,width)if p else ''for p,width in zip(r,crc_widths)]
                        groups=[''.join(payloads[i]for i in order)]if group else[payloads[i]for i in order]
                        assert all(len(p)%8==0 for p in groups)
                        coded=[]
                        for p in groups:
                            encoded=encode(packed(p),2)
                            if corrected:encoded,_=corrupt(encoded,len(p)//8,2)
                            coded.append(''.join(f'{b:08b}'for b in encoded))
                        selector=protected_header(field(index,4),True)
                        h=protected_header('000',True);stuff=(-len(selector)-len(h)-sum(map(len,coded)))%8;h=protected_header(field(stuff,3),True)
                        body=''.join(coded)
                        if kind==1:
                            body=''
                            for c in reversed(coded):body=''.join(octet_stage(list(c),list(body)))
                        wire=selector+h+body+'0'*stuff if kind==0 else interleave(selector,interleave(h,body+'0'*stuff,len(h)),len(selector))
                        packet=packed(wire);packets.append(packet);original=base['frames'][index];data=raw[original['plain_offset']:original['plain_offset']+original['plain_bytes']]
                        frames.append(dict(offset=len(blob),bytes=len(packet),plain_offset=len(plain),plain_bytes=len(data),class_bits=r,choice=index));blob.extend(packet);plain.extend(data)
                    case=dict(name=label,n=base['n'],channels=base['channels'],rate=base['rate'],asc=asc.hex(),frames=frames,container_rate=base['rate'],container_frame_samples=base['container_frame_samples'],samples=base['samples'],pcm_offset=0,slots=base['slots'],bands=base['bands'],golden=golden,k=2,group=group,first_fec=2 if group else 1,direct_mapping=True,video=label+'.mp4',matroska=label+'.mka')
                    files[label+'.asc']=asc;files[label+'.mp4']=video_fixture([case],blob,base['channels'],filename=label+'.mp4',write=False)['data'];files[label+'.mka']=mkv(asc,packets,base['rate'],base['container_frame_samples'],base['channels']);cases.append(case)

def generate():
    files={};cases=[];bad=[];blob=bytearray();plain=bytearray()
    for ld in[False,True]:
        path=SOURCE/('aac-eld-sbr'if ld else'aac-eld');m=json.loads((path/'manifest.json').read_text());source=(path/'packets.bin').read_bytes()
        for n in[480,512]:
            for channels in[1,2]:
                base=next(c for c in m['cases']if c['name']==f'n{n}-c{channels}'+('-x2-crc1'if ld else ''))
                # These authored ASCs end in epConfig=0. Rebuild the exact
                # pre-protection ELD prefix without guessing storage padding.
                from generate_he_aac_packet_fixtures import frequency
                from generate_aac_sbr_dsp_fixtures import header as sbr_header
                prefix=field(31,5)+field(7,6)+frequency(24000)+field(channels,4)+field(n==480,1)+'000'+field(ld,1)+('11'+sbr_header(0,True)if ld else '')+'0000'+'10'
                golden=f'n{n}-c{channels}-ld{int(ld)}.f32le';files[golden]=(path/(base['name']+'.f32le')).read_bytes()
                for group in[False,True]:
                    for kind,switch in[(0,0),(1,0),(2,0),(2,1),(2,3)]:
                        for cut in[1,3,5,7]:
                            until=cut in[1,5];reverse=(kind+cut)%2==0;k=2
                            ep,order,widths=config(kind,switch,group,cut,reverse,until,k);asc=packed(prefix+ep)
                            label=f'n{n}-c{channels}-ld{int(ld)}-g{int(group)}-i{kind}-s{switch}-b{cut}'
                            variants=['clean']+(['corrected','bad-crc','truncated','bad-sum']if kind==2 and switch==1 and cut==3 else [])+(['empty-tail']if group and kind==2 and switch==1 and cut==1 else [])
                            for variant in variants:
                                name=label+'-'+variant;rows=[];packets=[]
                                for index,row in enumerate(base['frames']):
                                    raw=source[row['offset']:row['offset']+row['bytes']]
                                    mutation=variant if variant=='corrected'or index==3 and variant!='clean'else None
                                    wire,parts=protect(raw,kind,switch,group,cut,reverse,until,k,mutation)
                                    rows.append(dict(offset=len(blob),bytes=len(wire),plain_offset=len(plain),plain_bytes=len(raw),class_bits=parts));blob.extend(wire);plain.extend(raw);packets.append(wire)
                                case=dict(name=name,n=n,channels=channels,rate=base['rate'],asc=asc.hex(),frames=rows,container_rate=base['rate'],container_frame_samples=base['container_frame_samples'],samples=base['samples'],pcm_offset=0,slots=base['slots'],bands=base['bands'],golden=golden,k=k,group=group,cut=cut,reverse=reverse,until_end=until,interleave=kind,switch=switch,first_fec=2 if group and order[0]==0 else 1,video=name+'.mp4',matroska=name+'.mka')
                                files[name+'.asc']=asc;files[name+'.mp4']=video_fixture([case],blob,channels,filename=name+'.mp4',write=False)['data'];files[name+'.mka']=mkv(asc,packets,base['rate'],base['container_frame_samples'],channels)
                                if variant in['clean','corrected']:cases.append(case)
                                else:case.update(good=label+'-clean',error={'bad-crc':'EP class CRC mismatch','truncated':'EP RS class/packet length mismatch','bad-sum':'EP RS information plus CRC is not octet aligned','empty-tail':'EP RS class/packet length mismatch'}[variant]);bad.append(case)
    ep3_cases(files,cases,blob,plain)
    files['class-vectors.json']=(json.dumps(dict(vectors=primitive_vectors(),provenance='Own bit strings and CRC; independent GF(256) polynomial division and deterministic correctable byte errors.'),indent=2)+'\n').encode()
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,provenance='Own ELD/LD-SBR sources and independent PCM; exact bit partitions, CRC-before-FEC, independent ascending RS polynomial division and octet interleaving. No private media, foreign codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
