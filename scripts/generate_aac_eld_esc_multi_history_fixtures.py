"""Own EP3 multi-AU header history and RS composition movies, offline.

Plain logical AUs are the composition reference, not independent numeric PCM.
The existing LD-SBR scalar corpus remains the numerical acceptance reference.
"""
import json
from pathlib import Path
from generate_aac_eld_fixtures import generate as core,mkv
from generate_aac_ep_concatenated_fixtures import config,protect
from generate_aac_sbr_history_fixtures import row as entropy
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_sbr_extension_fixtures import crc
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-eld-esc-multi-history'
_,HIGH,_,NOISE=tables(16,32,2,True,0,2)
NH=len(HIGH)-1;NQ=len(NOISE)-1

def header(coarse):
    # Absent extra fields select normative defaults (scale=2, alter=1, noise=2).
    return field(coarse,1)+field(8,4)+field(14,4)+'000'+'00'+'00'

def syntax(second,coarse,changed):
    count=2 if second else 1;noise_count=2 if second else 1
    # With transient position zero, the two-envelope grid has a seven-bit
    # spelling. Reserved CPE nibbles compensate exactly for an omitted header.
    grid='1000011'if second else'00001'
    controls=('1'+'0000'*2 if second else'0')+'0'+grid*2+'0'*((count+noise_count)*2)
    assert len(controls)+(0 if second else len(header(coarse)))==32
    envelopes=[];noises=[];left=changed;step=0
    for channel in range(2):
        envelope=''
        for env in range(count):
            values=[1 if second and coarse else 2]+[0]*(NH-1)
            for band in range(1,NH):
                if left:
                    values[band]=1 if step%2==0 else -1;step+=1;left-=1
            envelope+=entropy(values,False,second and coarse,False,False)
        envelopes.append(envelope)
        noises.append(''.join(entropy([7]+[0]*(NQ-1),False,False,False,True)for _ in range(noise_count)))
    data='00'*(2*NQ)+''.join(envelopes)+''.join(noises)+'00'
    prefix=field(not second,1)+(''if second else header(coarse))+controls
    assert len(prefix)==33
    return prefix,data

def sbr_patterns():
    patterns={}
    for second in [False,True]:
        for coarse in [False,True]:
            # Vary legal paired frequency deltas until the extension byte area
            # can make each coefficient class exactly the same bit length.
            prefix,data=next(syntax(second,coarse,n)for n in range(2*(NH-1)*(2 if second else 1)+1)if (len(syntax(second,coarse,n)[1])+5)%8==0)
            patterns[second,coarse]=(prefix,data)
    target=max(len(data)+5 for prefix,data in patterns.values())
    result={}
    for key,(prefix,data)in patterns.items():
        count=(target-len(data)-5)//8
        assert 0<=count<15
        # Reserved extension-id zero, with explicit nonzero opaque tail bytes.
        # The tail makes stale-header parsing fail instead of coincidentally
        # interpreting spare zero bytes as a different valid coefficient stream.
        data+='1'+field(count,4)+('00000000'+'11111111'*(count-1)if count else'')
        assert len(data)==target
        result[key]=[dict(category=9,bits=field(crc(prefix+data),10)+prefix),dict(category=10,bits=data)]
    return result

def generate():
    originals=core(True,[(n,24000,2,'base',0,0,False)for n in [480,512]],True)
    bases=json.loads(originals['manifest.json'])['cases'];patterns=sbr_patterns()
    files={};cases=[];bad=[];blob=bytearray();information=bytearray();plain=bytearray()
    for base in bases:
        if base['tool']!='base':continue
        original=base['frames'][0];n=base['n'];rate=48000
        setup=field(31,5)+field(7,6)+frequency(24000)+field(2,4)+field(n==480,1)+'000'+'111'+header(False)+'0000'
        for count in range(2,8):
            units=[];logical=[];details=[]
            for index in range(count*3):
                second=index%2!=0;coarse=(index//2)%2==0
                parts=[dict(p)for p in original['esc']]+[dict(p)for p in patterns[second,coarse]]
                logical.append(packed(original['bits']+''.join(p['bits']for p in parts if p['category']>=9)))
                parts[-1]['bits']+='0'*(-sum(len(p['bits'])for p in parts)%8)
                units.append(packed(''.join(p['bits']for p in parts)))
                details.append(dict(header_present=not second,coarse=coarse,parts=parts))
            lengths=[len(p['bits'])for p in details[0]['parts']]
            assert all([len(p['bits'])for p in d['parts']]==lengths for d in details)
            for family,kind in [('srcpc',0),('srcpc',1),('rs-bits',0),('rs-bits',1),('grouped',0),('grouped',1),('grouped',2)]:
                for flags in ['joined','separate','mixed']:
                    name=f'n{n}-f{count}-{family}-i{kind}-{flags}'
                    sets,ep=config(lengths,count,family,kind,flags);asc=packed(setup+'11'+ep+'1');files[name+'.asc']=asc
                    variants=['clean']+(['corrected']if family!='srcpc'and kind==0 and flags=='mixed'else [])
                    if count==7 and kind==0 and flags=='mixed':variants+=['late-sbr-crc','ep-crc','truncated']
                    for variant in variants:
                        label=name if variant=='clean'else name+'-'+variant;frames=[];packets=[]
                        for packet_index in range(3):
                            children=units[packet_index*count:(packet_index+1)*count].copy()
                            if variant=='late-sbr-crc'and packet_index==1:
                                bits=[p['bits']for p in details[(packet_index+1)*count-1]['parts']];at=next(i for i,p in enumerate(details[0]['parts'])if p['category']==9)
                                bits[at]=str(1-int(bits[at][0]))+bits[at][1:];children[-1]=packed(''.join(bits))
                            mutation={'ep-crc':'crc','truncated':'truncated','corrected':'corrected'}.get(variant)if packet_index==1 else None
                            packet,selected=protect(children,sets,kind,packet_index,mutation)
                            rows=[];logical_rows=[]
                            for j,child in enumerate(children):
                                index=packet_index*count+j
                                rows.append(dict(offset=len(information),bytes=len(child),header_present=details[index]['header_present'],coarse=details[index]['coarse']));information.extend(child)
                                raw=logical[index];logical_rows.append(dict(offset=len(plain),bytes=len(raw)));plain.extend(raw)
                            frames.append(dict(offset=len(blob),bytes=len(packet),choice=selected,units=rows,logical=logical_rows));blob.extend(packet);packets.append(packet)
                        c=dict(name=label,n=n,channels=2,configuration=2,rate=rate,source_frames=count,frame_samples=n*2,container_rate=rate,container_frame_samples=n*2*count,samples=n*2*count*3,pcm_offset=0,slots=n//32,bands=64,asc=asc.hex(),plain_asc=packed(setup+'00').hex(),frames=frames,configs=sets,video=label+'.mp4',matroska=label+'.mka')
                        files[c['video']]=video_fixture([c],blob,2,filename=c['video'],write=False)['data'];files[c['matroska']]=mkv(asc,packets,rate,n*2*count,2)
                        if variant in ['clean','corrected']:cases.append(c)
                        else:
                            c.update(good=name,bad_index=1,error='low-delay SBR CRC mismatch'if variant=='late-sbr-crc'else'EP class CRC mismatch'if variant=='ep-crc'else'EP class/packet length mismatch'if family=='srcpc'else'EP RS class/packet length mismatch');bad.append(c)
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(information);files['logical-packets.bin']=bytes(plain)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,padding=[],provenance='Own synthetic ELD core and SBR header/defaults, paired frequency deltas, reserved extension-id zero, independent EP CRC/polynomial RS/SRCPC/interleaving. Plain logical AUs are a composition reference, not independent numeric PCM. No private media, foreign codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
