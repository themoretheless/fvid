"""Own full/core/cross-boundary ELD ESC grouping; saved scalar PCM, offline."""
import json,re
from generate_mp4_ep1_fixtures import SOURCE
from generate_mp4_ep1_epochs_fixtures import eld_control
from generate_mp4_ep1_grouped_sbr_fixtures import grouped
OUT=SOURCE/'mp4-ep1-grouped-core'
def combined_controls():
    # Explicit TNS + RVLC/HCR controls also exercise the logical/transport
    # order difference. The independent scalar core and direct CLDFB oracle
    # generate PCM, never the production decoder.
    import struct
    from generate_aac_eld_fixtures import generate as core_generate
    from generate_aac_eld_sbr_fixtures import analysis,reconstruct,sbr_bits
    from generate_aac_eld_cldfb_fixtures import window,synthesis
    from generate_aac_sbr_frequency_oracles import tables
    from generate_aac_sbr_dsp_fixtures import header
    from generate_he_aac_packet_fixtures import field,frequency,packed
    from generate_mp4_ep1_fixtures import make_movie,ROOT
    source=core_generate(True,[(n,24000,1,'tns',flags,0,False)for n in [480,512]for flags in [3,7]],True)
    c=window();noise=[complex(*struct.unpack_from('<dd',(ROOT/'tests/fixtures/playback-errors/aac-sbr-noise-protocol.f64le').read_bytes(),i*16))for i in range(512)]
    _,high,low,_=tables(10,27,0,False,0,0)
    bases=json.loads(source['manifest.json'])['cases']
    for base,empty in [(b,False)for b in bases]+[(next(b for b in bases if b['n']==n),True)for n in [480,512]]:
        n=base['n'];flags=0 if empty else base['flags'];name=f'n{n}-c1-empty-core-x2-crc1'if empty else f'n{n}-c1-tns-resilience-f{flags}-x2-crc1'
        original=[0.]*(6*n)if empty else list(struct.unpack('<'+'f'*(6*n),source[base['name']+'.f32le'][:6*n*4]))
        samples=[v/32768 for v in synthesis(reconstruct(analysis(original,c),n//32,noise),64,c)]
        gold=struct.pack('<'+'f'*len(samples),*samples)
        asc=packed(field(31,5)+field(7,6)+frequency(24000)+field(1,4)+field(n==480,1)+field(flags,3)+'111'+header(0,True)+'0000'+'01')
        blob=bytearray();rows=[]
        for i,row in enumerate(base['frames'][:6]):
            core_parts=[dict(category=1,bits=field(140,8)+field(0,6)+'0'),dict(category=4,bits='')]if empty else[dict(p)for p in row['esc']]
            parts=core_parts+sbr_bits(1,len(high)-1,len(low)-1,n//32,i,True,categories=True)
            total=sum(len(p['bits'])for p in parts);parts[-1]['bits']+='0'*(-total%8)
            units=[]
            for j,p in enumerate(parts):
                bits=p['bits'];data=bytearray(packed(bits))
                if len(bits)%8:data[-1]|=(1<<(8-len(bits)%8))-1
                units.append(dict(stream_id=j+1,depends_on=j if j else None,offset=len(blob),bytes=len(data),bits=len(bits),timescale=24000,duration=n));blob.extend(data)
            rows.append(dict(units=units))
        fixture=dict(base,asc=asc.hex(),frames=rows)
        movie=make_movie(fixture,source[base['video']],blob,allow_empty=True,frame_count=6,file_dependencies=True)
        yield dict(base,name=name,flags=flags,container_rate=48000),movie,gold,len(rows[0]['units']),1

def generate():
    parent=SOURCE/'aac-eld-esc-sbr';m=json.loads((parent/'manifest.json').read_bytes())
    selected=[c for c in m['cases']if c['flags']or '-tns-'in c['name']or re.fullmatch(r'n(480|512)-c[1-7]-x[12]-crc[01]',c['name'])]
    files={};cases=[]
    controls=[]
    for base in selected:
        name=base['name'];controls.append((base,*eld_control(base['n'],base['configuration'],name.endswith('crc1'),'-x2-'in name,name)))
    controls.extend(combined_controls())
    for base,control,gold,count,channels in controls:
        name=base['name'];n=base['n'];cfg=base['configuration'];crc=name.endswith('crc1')
        groups={1:1,2:1,3:2,4:3,5:3,6:3,7:4}[cfg];core=count-2*groups
        if not re.fullmatch(r'n(480|512)-c[1-7]-x[12]-crc[01]',name):
            control_name=name+'-control.mp4';gold_name=name+'.f32le';files[control_name]=control;files[gold_name]=gold
        else:
            control_name='../mp4-ep1-grouped-sbr/'+name+'-control.mp4';gold_name='../mp4-ep1-grouped-sbr/'+name+'.f32le'
        variants={'whole':[list(range(count))], 'core':[list(range(core))]+[[i]for i in range(core,count)], 'zigzag':[[i for i in range(j,min(j+2,count))]for j in range(0,count,2)]}
        for mode,parts in variants.items():
            video=name+'-'+mode+'.mp4';files[video]=grouped(control,groups,'custom',partitions=parts)[0];malformed=[]
            if mode=='whole':
                for mutation,error in [('truncate','missing AAC ESC instance'),('extra','unconsumed AAC ESC information bits')]+([('crc','low-delay SBR CRC mismatch')]if crc else[]):
                    bad=name+'-'+mode+'-'+mutation+'.mp4';files[bad]=grouped(control,groups,'custom',mutation,parts)[0];malformed.append(dict(video=bad,error=error))
            cases.append(dict(name=name+'-'+mode,video=video,control=control_name,golden=gold_name,rate=base['container_rate'],channels=channels,n=n,streams=len(parts),original_streams=count,groups=groups,mode=mode,partitions=parts,flags=base['flags'],malformed=malformed))
    assert len(controls)==182,len(controls)
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own six-frame AOT39 core and LD-SBR instances grouped in transport order, including empty spectral categories, TNS and resilience combinations 1..7, including combined TNS/RVLC/HCR. Independent saved scalar native PCM. No private source media, codec process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
