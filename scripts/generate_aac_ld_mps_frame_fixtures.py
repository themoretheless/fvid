"""Own LD-MPS frame grammar corpus, explicit offline generation only.
No private media/parameters or foreign encoder; not spatial PCM acceptance.
"""
import json
from pathlib import Path
from generate_aac_ld_mps_config_fixtures import TREES, BANDS, length
from generate_aac_ld_mps_entropy_fixtures import field, packed, pcm, quant
from generate_aac_ld_mps_parameter_fixtures import maps
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT/'tests/fixtures/playback-errors/aac-ld-mps-frame'
DEFAULTS = [[15,15,0,0,15],[15,0,15,15,15],[15]*3,[15]*5,[15]*5,[15]*2,[15]*2,[15]]
SHAPE = [[5,5,4,6,6,4,4,2],[5,5,5,7,7,4,4,2]]
def descriptor(id, kind, start, stop, default, predictive=False):
    return dict(id=id,kind=kind,start=start,bands=stop-start,default=default,predictive=predictive)
def fine(v, kind, coarse):
    if not coarse:return v
    if kind=='CLD' and abs(v)==7:return 15 if v>0 else -15
    return v*2
def generate():
    book=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-frame/reshape-codewords.json').read_text());configs=bytearray();packets=bytearray();cases=[]
    for tree in range(8):
      for resolution in range(1,8):
       for variant in range(6):
        p=BANDS[resolution];channels,outputs,lfe,ttt_count=TREES[tree];time=[1,8,15,16,31,32][variant];shape=variant%3;one=variant%2 if tree!=7 else False;raw_downmix=variant%2==1;ext_kinds=[[],[4],[2],[0,1],[0,14],[2,4]][variant]
        ott=[(0 if variant==0 else p//2)if x else p for x in lfe]
        split=0 if variant==2 else p//2;dual=variant in [2,3,5];hi=(variant+1)%6
        wire=field(3,4)+field(time-1,5)+field(resolution,3)+field(tree,4)+field(variant%3,2)
        if tree!=7:wire+=field(one,1)
        wire+=field(raw_downmix,1)
        if tree!=7:wire+='000000'
        wire+='000'
        if tree!=7:wire+='0'
        wire+=field(shape,2)+'00'
        if tree!=7:wire+='0'
        for i,x in enumerate(lfe):
            if x:wire+=field(ott[i],5)
        for _ in range(ttt_count):wire+=field(dual,1)+field(variant,3)+(field(hi,3)+field(split,5)if dual else'')
        if shape==2:wire+='0'
        wire+='0'*(-len(wire)%8)
        extensions=[];extension_regions=[];config_extensions=[]
        for at,kind in enumerate(ext_kinds):
            regions=[]
            if kind==0:
                # Presence with zero bands exercises distinction from absent.
                data=field(3,4)+'00'+''.join('1'+field(0,5)if i%2==0 else'0'for i in range(len(lfe)+ttt_count))
            elif kind==1:data=field(3,4)+'00'+field(0,5)
            elif kind==2:
                zero=variant==5
                data='100'+'0'*(outputs-1)+'1'+field(zero,1)+(field(0,5)if zero else'')+''.join(field(i,5)for i in range(outputs+1))
                regions=[descriptor(f'arb:{at}:0','CLD',0,0 if zero else p,1)]
            else:data='10100101'
            payload=packed(data);config_extensions.append(dict(kind=kind,payload_position=len(wire)+4+len(length(len(payload)))));wire+=field(kind,4)+length(len(payload))+''.join(field(x,8)for x in payload)
            extensions.append(kind);extension_regions.append(regions)
        config=packed(wire);config_offset=len(configs);configs.extend(config)
        regions=[descriptor(f'ott-cld:{i}','CLD',0,n,DEFAULTS[tree][i])for i,n in enumerate(ott)]
        if one:regions.append(descriptor('ott-icc:all','ICC',0,p,0))
        else:regions.extend(descriptor(f'ott-icc:{i}','ICC',0,ott[i],0)for i,x in enumerate(lfe)if not x)
        for box in range(ttt_count):
            for high,start,stop,mode in [(False,0,split if dual else p,variant),(True,split if dual else p,p,hi)]:
                if high and start==stop:continue
                for component in range(2):
                    kind='CPC'if mode<2 else'CLD';default=10 if kind=='CPC'else 15 if component==0 else 0
                    regions.append(descriptor(f'ttt-{kind.lower()}:{box}:{int(high)}:{component}',kind,start,stop,default))
                if mode<2:regions.append(descriptor(f'ttt-icc:{box}:{int(high)}','ICC',start,stop,0,True))
        dmx=[descriptor(f'downmix:{i}','CLD',0,p,0)for i in range(channels)]if raw_downmix or 1 in ext_kinds else[]
        all_regions=regions+dmx+[r for rs in extension_regions for r in rs];previous={r['id']:[r['default']]*r['bands']for r in all_regions}
        frames=[]
        for fi in range(3):
            sets=1 if time==1 else 2;variable=fi==1;slots=[time-1]if sets==1 else [0,max(1,time-2)]if variable else[(time*(i+1)+sets-1)//sets-1 for i in range(sets)];independent=fi!=1
            frame=field(variable,1)+field(sets-1,1)+(''.join(field(x,(time-1).bit_length())for x in slots)if variable else'')+field(independent,1)
            expected=[];modes_used=[];mode_fields=[];reshape_terms=[];extension_lengths=[]
            def block(r,index):
                n=r['bands'];kind=r['kind'];coarse=(variant+index+fi)%2==1;stride_code=(tree+resolution+index+fi)%4 if n else 0;stride=[1,2,5,23][stride_code];_,groups=maps(n,stride)if n else([],[])
                modes=[3]*sets if fi==0 or r['predictive'] and fi==2 else[1]*sets if fi==1 and (variant%2==0 or sets==1 or 0 in ext_kinds)else[2,3]if fi==1 else[0]*sets
                modes_used.append(modes)
                data=''.join(field(m,2)for m in modes);rows=[]
                if 3 in modes:
                    pair=modes==[3,3];levels,offset,_=quant(kind,coarse)
                    compact=[[(j*7+index*3+row*5+fi)%levels-offset for j in range(len(groups))]for row in range(1+pair)]
                    data+=field(pair,1)+field(coarse,1)+field(stride_code,2)+'1'+pcm([compact[row][j]+offset for j in range(len(groups))for row in range(1+pair)],levels)
                    expanded=[[fine(v,kind,coarse)for v,g in zip(row,groups)for _ in g]for row in compact]
                coded=0
                for mode in modes:
                    if mode==0:previous[r['id']]=[r['default']]*n
                    elif mode==3:previous[r['id']]=expanded[coded];coded+=1
                    rows.append(previous[r['id']][:])
                expected.append(dict(id=r['id'],kind=kind,start=r['start'],values=rows))
                return data
            for i,r in enumerate(regions):
                mode_fields.append(dict(position=len(frame),id=r['id'],predictive=r['predictive']))
                frame+=block(r,i)
            smooth=[]
            for ps in range(sets):
                mode=(variant+fi+ps)%4;t=(fi+ps)%4 if mode>=2 else None;sc=(fi+ps)%4 if mode==3 else None;stride=[1,2,5,23][sc]if sc is not None else None;mask=[(g+fi+ps)%2==0 for g in range((p+stride-1)//stride)]if stride else[]
                frame+=field(mode,2)+(field(t,2)if t is not None else'')+(field(sc,2)+''.join(field(x,1)for x in mask)if sc is not None else'');smooth.append(dict(mode=mode,time=t,stride=stride,groups=mask))
            temporal=dict(kind='disabled')
            if shape:
                enabled=fi==1;frame+=field(enabled,1)
                if enabled:
                    count=SHAPE[shape-1][tree];on=[i%2==0 for i in range(count)];frame+=''.join(field(x,1)for x in on)
                    if shape==1:temporal=dict(kind='stp',enabled=on)
                    else:
                        envelopes=[]
                        for ch,yes in enumerate(on):
                            values=[]
                            if yes:
                                while len(values)<time:
                                    value=(ch+len(values))%5;run=min(8,time-len(values));symbol=(value<<4)+(run-1);row=next(r for r in book if r[2]==symbol);reshape_terms.append(dict(position=len(frame),width=row[0],value=value,run=run,remaining=time-len(values)));frame+=field(row[1],row[0]);values += [value]*run
                            envelopes.append(values if yes else None)
                        temporal=dict(kind='ges',envelopes=envelopes)
            for i,r in enumerate(dmx):
                mode_fields.append(dict(position=len(frame),id=r['id'],predictive=False))
                frame+=block(r,len(regions)+i)
            frame+='0'*(-len(frame)%8);payloads=[];main_expected=expected[:]
            for at,kind in enumerate(extensions):
                child=''
                if kind==2:
                    for i,r in enumerate(extension_regions[at]):child+=block(r,len(regions)+len(dmx)+i)
                    payload=packed(child)
                elif kind==4:
                    count=256 if variant==1 and resolution==1 else 3;payload=bytes((i*7+fi)%256 for i in range(count))
                elif kind in [0,1]:payload=bytes([0x80+fi])if kind==1 else b''
                else:payload=b''
                if kind<12:
                    extension_lengths.append(dict(position=len(frame),kind=kind,length=len(payload)))
                    frame+=field(min(255,len(payload)),8)+(field(len(payload)-255,16)if len(payload)>=255 else'')+''.join(field(x,8)for x in payload)
                payloads.append(dict(kind=kind,payload=payload.hex(),parameters=expected[len(main_expected):]if kind==2 else[]))
            packet=packed(frame);frames.append(dict(offset=len(packets),bytes=len(packet),bits=len(frame),slots=slots,independent=independent,parameters=main_expected,smoothing=smooth,temporal=temporal,extensions=payloads,mode_fields=mode_fields,reshape_terms=reshape_terms,extension_lengths=extension_lengths,leading_interpolation=any(m[0]==2 for m in modes_used),opaque_residual=1 in extensions));packets.extend(packet)
        cases.append(dict(name=f'tree{tree}-res{resolution}-v{variant}',config_offset=config_offset,config_bytes=len(config),config_extensions=config_extensions,tree=tree,variant=variant,bands=p,frame_slots=time,regions=regions+dmx,extension_regions=extension_regions,frames=frames))
    errors=[];error_packets=bytearray()
    c=next(c for c in cases if c['name']=='tree0-res1-v2');f=c['frames'][1]
    term=next(t for t in f['reshape_terms']if t['remaining']<8)
    wire=''.join(field(x,8)for x in packets[f['offset']:f['offset']+f['bytes']])[:f['bits']]
    row=next(r for r in book if r[2]==(term['value']<<4)+7)
    damaged=wire[:term['position']]+field(row[1],row[0])+wire[term['position']+term['width']:]
    packet=packed(damaged);errors.append(dict(name='ges-run-overflow',case=c['name'],seed=1,offset=0,bytes=len(packet),bits=len(damaged),reason='reshape run exceeds frame slots'));error_packets.extend(packet)
    c=next(c for c in cases if c['name']=='tree0-res1-v4');f=c['frames'][1]
    cfg=''.join(field(x,8)for x in configs[c['config_offset']:c['config_offset']+c['config_bytes']])
    at=c['config_extensions'][0]['payload_position']+7;cfg=cfg[:at]+field(1,5)+cfg[at+5:]
    wire=''.join(field(x,8)for x in packets[f['offset']:f['offset']+f['bytes']])[:f['bits']]
    at=f['mode_fields'][0]['position'];wire=wire[:at]+field(2,2)+wire[at+2:]
    packet=packed(wire);errors.append(dict(name='residual-interpolation',case=c['name'],config=packed(cfg).hex(),seed=1,offset=len(error_packets),bytes=len(packet),bits=len(wire),reason='residual coding forbids interpolated parameters'));error_packets.extend(packet)
    errors.append(dict(name='zero-band-residual-interpolation',case=c['name'],seed=1,offset=len(error_packets),bytes=len(packet),bits=len(wire),reason='residual coding forbids interpolated parameters'));error_packets.extend(packet)
    c=next(c for c in cases if c['name']=='tree0-res1-v0');f=c['frames'][0]
    source=''.join(field(x,8)for x in packets[f['offset']:f['offset']+f['bytes']])[:f['bits']]
    mode=next(x for x in f['mode_fields']if x['id']=='ott-cld:4')['position']
    # Grouped zero-width coding is accepted. A fine CLD PCM symbol still has
    # 31 levels, so codeword 31 is invalid even when its output range is empty.
    grouped=source[:mode+4]+field(1,2)+source[mode+6:mode+7]+field(31,5)+source[mode+7:]
    empty_huffman=source[:mode+6]+'0'+source[mode+7:]
    for name,wire,reason in [('zero-width-grouped-invalid-pcm',grouped,'unused LD-MPS PCM codeword'),('empty-huffman',empty_huffman,'empty LD-MPS Huffman parameter vector')]:
        packet=packed(wire)
        errors.append(dict(name=name,case=c['name'],seed=0,offset=len(error_packets),bytes=len(packet),bits=len(wire),reason=reason));error_packets.extend(packet)
    (OUT/'errors.bin').write_bytes(error_packets)
    (OUT/'configs.bin').write_bytes(configs);(OUT/'packets.bin').write_bytes(packets);(OUT/'manifest.json').write_text(json.dumps(dict(scope='Spatial frame syntax and parameter indices only; no spatial PCM/movie/playback acceptance.',cases=cases,errors=errors),separators=(',',':'))+'\n')
    print(len(cases),'programmes',sum(len(c['frames'])for c in cases),'frames',len(packets),'frame bytes')
if __name__=='__main__':generate()
