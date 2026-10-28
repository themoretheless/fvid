"""Own LD-SBR downscale movies, direct spectral/core/filterbank PCM oracle.
No private media, foreign decoder, subprocess or network. Generate explicitly.
"""
import json,struct
from functools import lru_cache
from generate_aac_eld_fixtures import generate as core,mkv
from generate_aac_eld_short_fixtures import short_window
from generate_aac_eld_downscale_info_fixtures import prefix,extension
from generate_aac_eld_cldfb_downscaled_fixtures import ROOT,analysis,synthesis
from generate_aac_eld_sbr_fixtures import reconstruct,sbr_bits
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_ld_layout_fixtures import LAYOUTS
from generate_he_aac_packet_fixtures import packed,video_fixture
SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-eld-sbr-downscaled'
@lru_cache(None)
def core_data(n,cfg,factor,tool='base',flags=0,ms=0):
    return core(True,specs=[(n,24000,cfg,tool,flags,ms,False)],output_samples=n//factor,synthesis_window=short_window if factor!=1 else None)
def core_pcm(n,cfg,factor,tool='base',flags=0,ms=0):
    data=core_data(n,cfg,factor,tool,flags,ms)
    case=json.loads(data['manifest.json'])['cases'][0];raw=data[case['name']+'.f32le']
    return struct.unpack('<'+'f'*(len(raw)//4),raw)
def generate(factors=(2,4), high_rates=(48000,)):
    parent=SOURCE/'aac-eld-sbr';manifest=json.loads((parent/'manifest.json').read_bytes());blob=(parent/'packets.bin').read_bytes()
    noise=[complex(*struct.unpack_from('<dd',(SOURCE/'aac-sbr-noise-protocol.f64le').read_bytes(),i*16))for i in range(512)]
    files={};cases=[];exports=[];output=bytearray()
    selected=manifest['cases']
    for base in selected:
      for factor in factors:
        n=base['n'];cfg=base['configuration'];channels=base['channels'];tool=base.get('core_tool','base');flags=base.get('resilience',0);ms=2 if tool=='ms-all'else 1 if tool in ['pns','correlated-pns','intensity','tns']else 0
        mode='base'if tool!='base'else base['name'].split('-c'+str(cfg)+'-',1)[1]
        if mode.startswith('x'):mode='base'
        hint=24000//factor;rate=base['rate']//factor;bands=base['bands']//factor
        head,tail=prefix(base['asc']);asc=packed(head+extension(hint,True)+tail);name=base['name']+f'-ds{factor}'
        elements,mapping,_=LAYOUTS[cfg];lfe=mapping[sum(2 if e==1 else 1 for e in elements[:elements.index(3)])]if 3 in elements else None
        short=core_pcm(n,cfg,factor,tool,flags,ms);full=core_pcm(n,cfg,1,tool,flags,ms);planar=[]
        for ch in range(channels):
            # Full companion retains coded LF bins above the short analysis width.
            use_full=ch==lfe or 10>32//factor
            pcm=full[ch::channels]if use_full else [v*factor for v in short[ch::channels]]
            low=[[v*32768 for v in row]for row in analysis(pcm,32 if use_full else 32//factor)]
            rows=[r+[0j]*32 for r in low]if ch==lfe else reconstruct([r[:10]for r in low],n//32,noise,mode,ch)
            planar.append([v/32768 for v in synthesis(rows,bands)])
        golden=b''.join(struct.pack('<'+'f'*channels,*(p[i]for p in planar))for i in range(len(planar[0])))
        frames=[];packets=[]
        source_case=json.loads(core_data(n,cfg,1,tool,flags,ms)['manifest.json'])['cases'][0]
        _,hb,lb,_=tables(10,27,0,False,0,0)
        protected='crc1'in base['name'] or mode!='base'
        for i,row in enumerate(base['frames']):
            packet=blob[row['offset']:row['offset']+row['bytes']]
            payload=source_case['frames'][i]['bits']+''.join(sbr_bits(2 if e==1 else 1,len(hb)-1,len(lb)-1,n//32,i,protected,mode)for e in elements if e!=3)
            assert packed(payload)==packet
            frames.append(dict(offset=len(output),bytes=len(packet),end_bits=len(payload)));output.extend(packet);packets.append(packet)
        case=dict(base,name=name,asc=asc.hex(),frames=frames,rate=rate,container_rate=rate,container_frame_samples=n*base['bands']//64*2//factor,samples=len(planar[0]),factor=factor,source_rate=base['rate'],out_n=n*base['bands']//32//factor,video=name+'.mp4',matroska=name+'.mka',golden=name+'.f32le',control='../aac-eld-sbr/'+base['video'])
        files[case['golden']]=golden;files[case['video']]=video_fixture([case],output,channels,filename=case['video'],write=False)['data'];files[case['matroska']]=mkv(asc,packets,rate,case['out_n'],channels);cases.append(case)
    # 48-kHz core / 96-kHz syntax: start=0 -> k0=5, stop=14 -> k2=10.
    # Master [5,6,7,8,10]: patch (source=1,target=5,width=3),
    # final two-band patch discarded by the protocol. This exercises analysis8.
    # The optional 96-kHz core has k0=4,k2=8 and a retained 2->4 width-2
    # patch, followed by a discarded width-2 patch; it exercises analysis4.
    from generate_aac_sbr_dsp_fixtures import header
    from generate_he_aac_packet_fixtures import field,frequency
    for core_rate in high_rates:
     assert core_rate in (48000,96000)
     kx,stop,start,patch_sources=(5,10,0,[1,2,3,None,None])if core_rate==48000 else (4,8,3,[2,3,None,None])
     _,high,low,_=tables(kx,stop,0,False,0,0)
     h=header(0,True);h=h[:1]+field(start,4)+field(14,4)+h[9:]
     for n in [480,512]:
       for cfg in [1,2]:
        for factor in factors:
         source=core(True,[(n,core_rate,cfg,'base',0,0,False)],output_samples=n//factor,synthesis_window=short_window)
         base=json.loads(source['manifest.json'])['cases'][0];raw=source[base['name']+'.f32le'];channels=base['channels'];pcm=struct.unpack('<'+'f'*(len(raw)//4),raw)
         use_full=kx>32//factor
         full=core(True,[(n,core_rate,cfg,'base',0,0,False)])if use_full else None
         raw_full=full[base['name']+'.f32le']if use_full else None;nominal=struct.unpack('<'+'f'*(len(raw_full)//4),raw_full)if use_full else None
         rows=[reconstruct([[v*32768 for v in row[:kx]]for row in analysis(nominal[ch::channels]if use_full else [v*factor for v in pcm[ch::channels]],32 if use_full else 32//factor)],n//32,noise,geometry=(kx,stop,patch_sources))for ch in range(channels)]
         for double in [False,True]:
             rate=core_rate*(1+double)//factor;bands=(64 if double else 32)//factor;name=f'n{n}-r{core_rate}-c{cfg}-x{1+double}-ds{factor}'
             asc=packed(field(31,5)+field(7,6)+frequency(core_rate)+field(cfg,4)+field(n==480,1)+'000'+'1'+field(double,1)+'1'+h+extension(core_rate//factor,True)+'0000'+'00')
             frames=[];packets=[]
             for i,row in enumerate(base['frames']):
                 packet=packed(row['bits']+sbr_bits(channels,len(high)-1,len(low)-1,n//32,i,True));frames.append(dict(offset=len(output),bytes=len(packet),end_bits=len(row['bits']+sbr_bits(channels,len(high)-1,len(low)-1,n//32,i,True))));output.extend(packet);packets.append(packet)
             planar=[[v/32768 for v in synthesis(r,bands)]for r in rows];golden=b''.join(struct.pack('<'+'f'*channels,*(p[i]for p in planar))for i in range(len(planar[0])))
             case=dict(base,name=name,asc=asc.hex(),frames=frames,rate=rate,container_rate=rate,container_frame_samples=n*(1+double)//factor,samples=len(planar[0]),factor=factor,source_rate=core_rate*(1+double),out_n=n*(1+double)//factor,video=name+'.mp4',matroska=name+'.mka',golden=name+'.f32le',short_analysis8=factor==4)
             files[case['golden']]=golden;files[case['video']]=video_fixture([case],output,channels,filename=case['video'],write=False)['data'];files[case['matroska']]=mkv(asc,packets,rate,case['out_n'],channels);cases.append(case)
    from generate_mp4_ep1_grouped_ps_fixtures import replace_asc
    ep=json.loads((SOURCE/'aac-eld-esc-sbr'/'manifest.json').read_bytes());ep_blob=(SOURCE/'aac-eld-esc-sbr'/'packets.bin').read_bytes()
    for base in [c for c in cases if 'short_analysis8'not in c]:
        stem=base['name'].rsplit('-ds',1)[0];protected=next(c for c in ep['cases']if c['name']==stem)
        head,tail=prefix(protected['asc']);asc=packed(head+extension(24000//base['factor'],True)+tail)
        frames=[];packets=[]
        for row in protected['frames']:
            packet=ep_blob[row['offset']:row['offset']+row['bytes']];frames.append(dict(offset=len(output),bytes=len(packet)));output.extend(packet);packets.append(packet)
        name='ep3-'+base['name'];case=dict(base,name=name,asc=asc.hex(),frames=frames,video=name+'.mp4',matroska=name+'.mka',protected=True)
        files[case['video']]=video_fixture([case],output,case['channels'],filename=case['video'],write=False)['data'];files[case['matroska']]=mkv(asc,packets,case['rate'],case['out_n'],case['channels']);cases.append(case)
        original=next(c for c in manifest['cases']if c['name']==stem);head,tail=prefix(original['asc']);asc=packed(head+extension(24000//base['factor'],True)+tail[:4]+'01'+tail[6:]);name='ep1-'+base['name'];case=dict(base,name=name,asc=asc.hex(),video=name+'.mp4',control=base['video'])
        files[case['video']]=replace_asc(files[base['video']],asc);exports.append(case)
    files['packets.bin']=bytes(output);files['manifest.json']=(json.dumps(dict(cases=cases,exports=exports,provenance='Own nonzero coded LD-SBR AUs unchanged; independently solved short core windows, direct cosine, time convolution and modulated-column oracle. Full centered analysis companion when coded crossover exceeds short analysis width. No private media, foreign decoder, subprocess or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
