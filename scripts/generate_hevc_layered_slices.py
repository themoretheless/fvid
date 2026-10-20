#!/usr/bin/env python3
"""Owned Annex F header witnesses. Entropy is an opaque marker, not coded video.
The authored Y4M atlas supplies actual source images for header-to-DPB binding.
No external codec, private source, FFmpeg or network is used.
"""
import itertools,json
from pathlib import Path
import generate_hevc_inter_layer_sets as v
import generate_hevc_layered_sps as g
from generate_hevc_layered_pps import se
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
ue=g.ue
ENTROPY=b'\xa5\x5a\x80'
def nal(kind,layer,tid,bits):return ((kind<<9)|(layer<<3)|(tid+1)).to_bytes(2,'big')+g.escape(g.packed(bits))
def header_pps(layer,extra,weighted,reset=False,depth_tables=None,constrained=False,parallel_merge=2,deblock=False):
    b=ue(0)+ue(0)+'11'+f'{extra:03b}'+'01'+ue(0)+ue(0)+se(0)+str(int(constrained))+'00'+se(0)+se(0)
    b+='0'+str(int(weighted))*2+'000'+'0'+('100'+se(0)+se(0)if deblock else'101')+'0'+'1'+ue(parallel_merge-2)+'1'
    b+='1'+('01100000'if depth_tables is not None else'01000000')+str(int(reset))+'0'+ue(0)+'0'+(depth_tables or '')+'1'
    return nal(34,layer,0,b)
def main():
    configs={
        'single-layer':dict(single_layer=True),
        'aligned':dict(aligned=True),
        'explicit':dict(), 'default':dict(default=True),
        'default-filtered':dict(default=True,sublayers=[0,1,2,2]),
        'filtered':dict(limits=[0,1,2]), 'one':dict(one=True),
        'default-bad-one':dict(default=True,one=True),
        'independent':dict(independent=True,poc_absent=False),
        'absent-poc':dict(independent=True,poc_absent=True),
    }
    cases=[]
    def add(label,config='explicit',layer=9,kind=1,tid=0,stype=1,enabled=True,count=2,indices=None,temporal=False,extra=2,discardable=False,cross=False,weighted=False,refs=(1,1),mods=None,mvp=False,error=None,binding_error=None,source_irap=False,next_options=None,dependent=False,missing_previous=False,reserved=None,reset=False,extension=None,reset_expected=None,poc=100):
        options=dict(locals());options.pop('label');options.pop('next_options')
        cfg=configs[config];independent=cfg.get('independent',False)
        prefix='hevc-layered-slice-'+label
        fmt=dict(size=[64,32],chroma=1,separate=False,depth=[8,8],crop=[0]*4)
        if cfg.get('single_layer'):
            bits='0000'+'11'+'000000'+'0101'+'1'*16+g.ptl(True)+'0'+ue(3)+ue(1)+ue(0)+'000000'+ue(0)+'0'+'0'+'1'
            raw=nal(32,0,0,bits)
        elif independent:
            raw,_,_=g.vps(fmt,independent=True,poc_absent=cfg['poc_absent'])
        else:raw=v.vps([4,9,1,6],size=[64,32],**cfg)
        vps_file='hevc-layered-slice-config-'+config+'.vps'
        (D/vps_file).write_bytes(raw)
        sps=g.sps(multilayer=not(independent or layer==0),independent=independent or layer==0,scaling='disabled',coded_size=(64,32),buffers=4,reorder=1)
        if not(independent or layer==0):sps=((33<<9)|(layer<<3)|1).to_bytes(2,'big')+sps[2:]
        sps_file=f'hevc-layered-slice-sps-layer{layer}-'+('full' if independent or layer==0 else 'multilayer')+'.sps'
        (D/sps_file).write_bytes(sps)
        pps_file=f'hevc-layered-slice-pps-layer{layer}-extra{extra}-weighted{int(weighted)}'+('-reset' if reset else '')+'.pps'
        (D/pps_file).write_bytes(header_pps(layer,extra,weighted,reset))
        direct=[] if independent or layer==0 else [0] if layer in (2,5) else [0,2,5]
        limits=cfg.get('limits',[7,7,7]);sub=cfg.get('sublayers',[2]*4)
        def fields(o,first=True,dep=False):
            n=len(direct);t=o['tid'];k=o['kind'];s=o['stype'];en=o['enabled']
            eligible=[i for i,id in enumerate(direct) if sub[v.IDS.index(id)]>=t and (t==0 or limits[v.IDS.index(id)]>t)]
            active=0 if not n or not eligible else len(eligible) if cfg.get('default') else 0 if not en else 1 if cfg.get('one') or n==1 else o['count']+1
            selected_indices=[] if active==0 else o['indices'] if o['indices'] is not None else eligible[:active]
            # Explicit IDCs are present whenever an enabled partial set is coded.
            coded=en and not cfg.get('default') and n>1 and active!=n
            if not coded:selected_indices=eligible[:active]
            selected=[direct[i] for i in selected_indices if i<n]
            poc_present=k not in (19,20) or layer>0 and not cfg.get('poc_absent',False)
            b=str(int(first))+('0' if 16<=k<=21 else '')+ue(0)
            if not first:b+=str(int(dep))+'1'
            if not dep:
                b+=str(int(o['discardable']))[:min(extra,1)]+(str(int(o['cross'])) if extra>1 else '')+(o['reserved'] if o['reserved'] is not None else '0'*max(0,extra-2))
                b+=ue(s)+'1' # picture output
                if poc_present:b+=f'{o["poc"]:08b}'
                if k not in (19,20):
                    b+='0'+ue(int(o['temporal']))+ue(0)+(ue(0)+'1' if o['temporal'] else '')
                    b+=ue(0)+ue(0)+str(int(o['mvp'])) # no long-term refs, temporal MVP
                flag=None;coded_count=None
                if layer>0 and not cfg.get('default') and n:
                    b+=str(int(en));flag=en
                    if en and n>1:
                        if not cfg.get('one'):b+=f'{o["count"]:0{(n-1).bit_length()}b}';coded_count=o['count']
                        if active!=n:
                            b+=''.join(f'{i:0{(n-1).bit_length()}b}' for i in selected_indices)
                b+='10' # SAO marker
                if s!=2:
                    override=o['refs']!=(1,1)
                    b+=str(int(override))
                    if override:b+=ue(o['refs'][0]-1)+(ue(o['refs'][1]-1) if s==0 else '')
                    total=int(o['temporal'])+active
                    if total>1:
                        for l in range(2 if s==0 else 1):
                            mod=(o['mods'] or [None,None])[l]
                            b+=str(int(mod is not None))
                            if mod is not None:b+=''.join(f'{i:0{(total-1).bit_length()}b}' for i in mod)
                    if s==0:b+='1'
                    b+='1' # CABAC init
                    if o['mvp'] and k not in (19,20):
                        if s==0:b+='0' # collocated L1
                        if o['refs'][1 if s==0 else 0]>1:b+=ue(1)
                    if o['weighted']:
                        b+=ue(1)+se(0)
                        for l in range(2 if s==0 else 1):
                            r=o['refs'][l]
                            b+=''.join(str(int(i%2==0)) for i in range(r))
                            b+=''.join(str(int(i%2==1)) for i in range(r))
                            for i in range(r):
                                if i%2==0:b+=se(0)+se(1)
                                else:b+=(se(0)+se(0))*2
                    b+=ue(2) # three merge candidates
                b+=se(1) # QP 27
            else:flag=None;coded_count=None
            ext=b'\x80' if k in (16,17,18,21) else b'\x00' if independent else b''
            if o['extension'] is not None:ext=bytes(o['extension'])
            b+=ue(len(ext))+''.join(f'{x:08b}' for x in ext)+'1'
            b+='0'*((-len(b))%8);offset=len(b)//8
            b+=''.join(f'{x:08b}' for x in ENTROPY)
            return nal(k,layer,t,b),dict(selected=selected,flag=flag,count=coded_count,poc_present=poc_present,entropy_offset=offset,extension=list(ext))
        raw,expected=fields(options)
        (D/(prefix+'.nal')).write_bytes(raw)
        if next_options is not None or dependent:
            second=dict(options);second.update(next_options or {})
            raw2,expected2=fields(second,first=False,dep=dependent)
            (D/(prefix+'.next.nal')).write_bytes(raw2)
            if dependent:
                for key in ('selected','flag','count','poc_present'):expected2[key]=expected[key]
            expected=expected2
        if reset_expected is not None:expected['reset']=reset_expected
        if poc != 100:expected['poc']=poc
        cases.append(dict(prefix=prefix,vps=vps_file,sps=sps_file,pps=pps_file,ols=0 if cfg.get('single_layer') else 1,config=config,layer=layer,kind=kind,tid=tid,stype=stype,extra=extra,discardable=discardable,cross=cross,weighted=weighted,refs=refs,mods=mods,mvp=mvp,temporal=temporal,expected=expected,error=error,binding_error=binding_error,source_irap=source_irap,next=next_options is not None or dependent,dependent=dependent,missing_previous=missing_previous))
    for stype,tid,count in itertools.product((0,1,2),(0,1,2),(0,1,2)):
        for chosen in itertools.combinations(range(3),count+1):
            add(f'explicit-s{stype}-t{tid}-c{count}-i'+''.join(map(str,chosen)),stype=stype,tid=tid,count=count,indices=list(chosen))
    for tid,stype in itertools.product((0,1,2),(0,1,2)):
        add(f'default-t{tid}-s{stype}',config='default',tid=tid,stype=stype)
        add(f'default-filtered-t{tid}-s{stype}',config='default-filtered',tid=tid,stype=stype)
    for chosen,tid in itertools.product(range(3),(0,1,2)):
        add(f'one-i{chosen}-t{tid}',config='one',count=0,indices=[chosen],tid=tid)
    add('disabled-intra',enabled=False,stype=2)
    add('disabled-temporal',enabled=False,temporal=True)
    add('disabled-inter-empty',enabled=False,error='no current references')
    add('filtered-t1',config='filtered',tid=1,count=0,indices=[2])
    add('filtered-t2-empty-intra',config='filtered',tid=2,stype=2,count=1,indices=[])
    add('filtered-t2-empty-inter',config='filtered',tid=2,count=1,indices=[],error='no current references')
    add('filtered-t1-inferred-missing',config='filtered',tid=1,stype=2,count=2,error='inferred inter-layer reference is absent')
    add('filtered-t1-bad-index',config='filtered',tid=1,count=0,indices=[1],error='temporally unavailable')
    add('zero-limit-needs-irap',config='filtered',count=0,indices=[0],binding_error='temporal admission')
    add('zero-limit-source-irap',config='filtered',count=0,indices=[0],source_irap=True)
    add('one-default-violation',config='default-bad-one',stype=2,error='violates VPS limit')
    add('count-range',count=3,indices=[],error='count exceeds direct layers')
    add('index-range',count=0,indices=[3],error='index/order')
    add('index-repeated',count=1,indices=[1,1],error='index/order')
    add('index-descending',count=1,indices=[2,0],error='index/order')
    add('bad-modification',mods=[[3],None],error='reference-list entry out of range')
    for kind,stype in itertools.product((19,20),(0,1,2)):add(f'dependent-idr{kind}-s{stype}',kind=kind,stype=stype)
    for config,kind in itertools.product(('independent','absent-poc'),(19,20)):
        add(f'{config}-idr{kind}',config=config,layer=3,kind=kind,stype=2,enabled=False,cross=True)
    add('cra-temporal-rps',kind=21,stype=2,temporal=True,error='IRAP cannot use temporal RPS')
    add('independent-bla-temporal-rps',config='independent',layer=3,kind=16,stype=2,temporal=True,error='IRAP cannot use temporal RPS')
    add('independent-trail',config='independent',layer=3,temporal=True,enabled=False)
    add('independent-irap-inter',config='independent',layer=3,kind=19,error='IRAP slice must be intra')
    add('cross-dependent-idr',kind=19,cross=True,error='independent IDR')
    add('cross-non-idr',cross=True,error='independent IDR')
    add('discardable-r',discardable=True,error='cannot be discardable')
    add('discardable-n',kind=0,discardable=True)
    add('reserved-extra',extra=7,reserved='10101')
    for stype in (0,1,2):add(f'single-layer-s{stype}',config='single-layer',layer=0,stype=stype,temporal=True,enabled=False)
    for extra in (0,1,3,7):add(f'extra-{extra}',extra=extra)
    for stype in (0,1):add(f'weighted-s{stype}',stype=stype,weighted=True,refs=(2,2),mods=[[2,0],[1,2]] if stype==0 else [[2,0],None],mvp=True)
    add('single-direct',layer=2)
    add('single-direct-disabled-intra',layer=2,enabled=False,stype=2)
    add('base-idr',layer=0,kind=19,stype=2)
    add('single-layer-inherited',config='single-layer',layer=0,temporal=True,enabled=False,dependent=True)
    add('base-inherited',layer=0,temporal=True,enabled=False,dependent=True)
    add('next-cross-layer-bla',config='independent',layer=3,kind=19,stype=2,enabled=False,next_options=dict(cross=True),error='disagrees with picture header')
    add('base-temporal',layer=0,temporal=True,enabled=False)
    add('inherited-dependent',dependent=True)
    add('dependent-missing-header',dependent=True,missing_previous=True,error='no preceding header')
    add('independent-next',next_options={})
    add('next-layer-flags',kind=0,next_options=dict(discardable=True),error='disagrees with picture header')
    add('next-layer-selection',count=0,indices=[0],next_options=dict(indices=[1]),error='disagrees with picture header')
    add('next-enabled-flag',stype=2,next_options=dict(enabled=False),error='disagrees with picture header')
    add('next-coded-count-empty',config='filtered',tid=2,stype=2,count=0,indices=[],next_options=dict(count=1),error='disagrees with picture header')
    def extension_bits(bits):
        bits+='0'*((-len(bits))%8)
        return [int(bits[i:i+8],2) for i in range(0,len(bits),8)]
    for kind in (16,17,18,21):
        for aligned in (False,True):
            for present in (False,True):
                # F-55: only aligned dependent CRA/BLA has an explicit flag.
                cycle_present=present if aligned else True
                bits=(str(int(present)) if aligned else '')+(ue(3) if cycle_present else '')
                add(f'reset-cycle-k{kind}-a{int(aligned)}-p{int(present)}',config='aligned' if aligned else 'explicit',kind=kind,extension=extension_bits(bits),reset_expected=dict(idc=0,period=None,lsb=None,cycle=3 if cycle_present else None))
    for idc in (1,2,3):
        for full in ((False,True) if idc==3 else (False,)):
            bits=f'{idc:02b}{7:06b}'+(str(int(full))+f'{100:08b}' if idc==3 else '')
            ext=extension_bits(bits)
            expected=dict(idc=idc,full=full,period=7,lsb=100 if idc==3 else None,cycle=None)
            add(f'reset-idc{idc}-f{int(full)}',reset=True,extension=ext,reset_expected=expected)
            add(f'reset-inherited-idc{idc}-f{int(full)}',reset=True,extension=ext,reset_expected=expected,dependent=True)
    for idc,full in ((1,False),(2,False),(3,False),(3,True)):
        for cycle in (None,3):
            bits=f'{idc:02b}{7:06b}'+(str(int(full))+f'{100:08b}' if idc==3 else '')
            bits+=str(int(cycle is not None))+(ue(cycle) if cycle is not None else '')
            add(f'reset-aligned-idc{idc}-f{int(full)}-cycle{cycle}',config='aligned',reset=True,extension=extension_bits(bits),reset_expected=dict(idc=idc,full=full,period=7,lsb=100 if idc==3 else None,cycle=cycle))
    add('reset-signalled-discardable',kind=0,discardable=True,reset=True,extension=extension_bits('11'+'000111'+'1'+f'{100:08b}'),reset_expected=dict(idc=3,full=True,period=7,lsb=100,cycle=None))
    add('reset-cycle-overflow',config='aligned',extension=extension_bits('1'+ue((1<<24)+1)),error='cycle out of range')
    add('reset-signalled-truncated',reset=True,extension=[0xc7],error='truncated or oversized bit field')
    add('reset-discardable',kind=0,discardable=True,reset=True,extension=[0x47],error='reset requires a reference picture')
    add('reset-temporal-id',tid=1,reset=True,extension=[0x47],error='reset requires a reference picture')
    add('reset-signalled-cra',kind=21,reset=True,extension=extension_bits('11'+'000111'+'0'+f'{100:08b}'),error='cannot use signalled POC reset')
    add('reset-truncated',reset=True,extension=[],error='truncated or oversized bit field')
    for dependent in (False,True):
        add(f'reset-mismatch-dependent{int(dependent)}',reset=True,extension=[0x47],next_options=dict(extension=[0x48]),dependent=dependent,error='disagrees with picture header')
    add('reset-absent-full-nonzero',config='absent-poc',layer=3,kind=19,stype=2,enabled=False,reset=True,extension=extension_bits('11'+'000111'+'1'+f'{100:08b}'),error='full signalled reset has nonzero absent-layer LSB')
    add('reset-absent-full-zero',config='absent-poc',layer=3,kind=19,stype=2,enabled=False,reset=True,extension=extension_bits('11'+'000111'+'1'+'00000000'),reset_expected=dict(idc=3,full=True,period=7,lsb=0,cycle=None))
    sequences=[]
    def state_au(label,aligned,poc,kind=1,idc=0,period=7,anchor=102,full=False,cycles=None,layers=(0,2,5,9),discard=False,mixed=False,expected_poc=None,deltas=None,previous=None,error=None,overrides=None,resetting=None):
        prefixes=[]
        for layer in layers:
            k=19 if mixed and layer==0 else kind
            values=dict(kind=k,idc=idc,period=period,poc=poc,cycle=cycles,full=full,anchor=anchor,tid=0)
            values.update((overrides or {}).get(layer,{}));k=values['kind'];r=values['idc'];cycle=values['cycle']
            b=f'{r:02b}'+(f'{values["period"]:06b}' if r else '')
            if r==3:b+=str(int(values['full']))+f'{values["anchor"]:08b}'
            required=k in (16,17,18,21) and (not aligned or layer==0)
            if not required and aligned:b+=str(int(cycle is not None))
            if required or cycle is not None:b+=ue(cycle if cycle is not None else 0)
            add('state-'+label+'-layer'+str(layer),config='aligned' if aligned else 'explicit',layer=layer,kind=k,tid=values['tid'],stype=2,enabled=False,discardable=discard,reset=True,poc=values['poc'],extension=extension_bits(b))
            prefixes.append(cases[-1]['prefix'])
        return dict(headers=prefixes,poc=expected_poc,decrements=deltas or {},previous=previous or {},error=error,resetting={str(i):(resetting if resetting is not None else idc in (1,2) and (not aligned or i==layers[0])) for i in layers})
    for aligned in (False,True):
        name='aligned' if aligned else 'unaligned';all_layers=(0,2,5,9)
        def au(tag,**kw):return state_au(name+'-'+tag,aligned,**kw)
        initial=au('initial',poc=100,kind=21,cycles=1,expected_poc=356,previous={str(i):356 for i in all_layers})
        normal=au('normal',poc=101,expected_poc=357,previous={str(i):357 for i in all_layers})
        reset=au('msb',poc=102,idc=1,expected_poc=102,deltas={str(i):256 for i in all_layers},previous={str(i):102 for i in all_layers})
        discard=au('discard',poc=103,kind=21,cycles=0,discard=True,expected_poc=103,previous={str(i):102 for i in all_layers})
        signalled=au('signalled',poc=104,idc=3,anchor=102,expected_poc=104,previous={str(i):104 for i in all_layers})
        sequences.append(dict(label=name+'-msb-discard-signalled',units=[initial,normal,reset,discard,signalled]))
        base=au('missing-base-msb',poc=102,idc=1,layers=(0,),expected_poc=102,deltas={str(i):256 for i in (all_layers if aligned else (0,))},previous={str(i):(102 if aligned or i==0 else 357) for i in all_layers})
        late=au('late-enhancement',poc=103,idc=3,anchor=102,layers=(2,5,9),expected_poc=103,resetting=not aligned,deltas={} if aligned else {str(i):256 for i in (2,5,9)},previous={str(i):(102 if i==0 else 103) for i in all_layers})
        sequences.append(dict(label=name+'-missing-reset-layers',units=[initial,normal,base,late]))
        full_reset=au('full',poc=0,idc=2,period=8,mixed=True,expected_poc=0,deltas={str(i):256 for i in all_layers},previous={str(i):0 for i in all_layers})
        full_signal=au('full-signalled',poc=1,idc=3,period=8,anchor=0,full=True,expected_poc=1,previous={str(i):1 for i in all_layers})
        sequences.append(dict(label=name+'-full-mixed-idr',units=[initial,full_reset,full_signal]))
        for tag,kw,error in [
            ('bad-idc',dict(poc=102,idc=1,overrides={2:dict(idc=0)}),'disagrees on temporal ID or POC reset'),
            ('bad-period',dict(poc=102,idc=1,overrides={2:dict(period=8)}),'disagrees on temporal ID or POC reset'),
            ('bad-poc',dict(poc=102,overrides={2:dict(poc=103)}),'derived POCs disagree'),
            ('bad-temporal',dict(poc=102,overrides={2:dict(tid=1)}),'disagrees on temporal ID or POC reset'),
            ('bad-base-idr',dict(poc=0,mixed=True),'mixed base IDR access unit requires full'),
            ('bad-base-irap',dict(poc=102,kind=21,cycles=1,overrides={2:dict(kind=1,cycle=None)}),'mixed base IRAP access unit requires'),
            ('bad-enhancement-idr',dict(poc=100,kind=21,cycles=1,overrides={2:dict(kind=19,cycle=None)}),'mixed enhancement IDR types require'),
        ]:
            bad=au(tag,**kw,error=error);sequences.append(dict(label=name+'-'+tag,units=[initial,normal,bad]))
        bad_full=au('bad-full-anchor',poc=104,idc=3,full=True,anchor=102,error='signalled reset disagrees with initial access unit')
        sequences.append(dict(label=name+'-bad-full-anchor',units=[initial,normal,reset,bad_full]))
        repeat=au('bad-repeat',poc=103,idc=1,error='initial resets reuse a period ID')
        sequences.append(dict(label=name+'-bad-repeat',units=[initial,normal,reset,repeat]))
    pictures=[];atlas=b'YUV4MPEG2 W64 H32 F25:1 Ip A1:1 C420\n'
    for au in (40,41):
        for i,layer in enumerate(v.IDS):
            planes=[[(au*3+i*31+x*7+y*11+c*29)%256 for y in range(32 if c==0 else 16) for x in range(64 if c==0 else 32)] for c in range(3)]
            pictures.append(dict(status=v.status(layer,au,100),planes=planes));atlas+=b'FRAME\n'+bytes(sum(planes,[]))
    (D/'hevc-layered-slice-atlas.y4m').write_bytes(atlas)
    (D/'hevc-layered-slices.json').write_text(json.dumps(dict(cases=cases,state_sequences=sequences,pictures=pictures,entropy=list(ENTROPY),atlas='hevc-layered-slice-atlas.y4m',provenance='Original complete VPS/SPS/PPS and slice-header NAL witnesses. Entropy bytes are opaque parser markers, not valid scalable picture data. Original eight-frame 64x32 Y4M atlas supplies source images for header-to-DPB binding. Header/binding acceptance only, not scalable playback; no private media, external codec, FFmpeg or network.'),separators=(',',':'))+'\n')
    print('Layered slice cases:',len(cases),flush=True)
if __name__=='__main__':main()
