#!/usr/bin/env python3
"""Own signalled 3D-stereo TTT inversion movies; explicit offline generation."""
import json,struct
from pathlib import Path
from generate_aac_eld_fixtures import mkv
from generate_aac_eld_downscale_info_fixtures import extension
from generate_aac_ld_mps_stp_fixtures import config
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_he_aac_packet_fixtures import packed,field,frequency,video_fixture
ROOT=Path(__file__).resolve().parents[1]
FIX=ROOT/'tests/fixtures/playback-errors'
OUT=FIX/'aac-ld-mps-3d'
FRAMES=12

def main():
    OUT.mkdir(exist_ok=True)
    models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases']
    configs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes()
    controls,cases,blob=[],[],bytearray()
    def rows(bits):
        out=[]
        for b in bits:
            wire=packed(b);out.append(dict(offset=len(blob),bytes=len(wire),core_bits=len(b)));blob.extend(wire)
        return out
    selections=[('aac-ld-mps-downscaled','aot39-n512-ds2-c2')]
    selections += [('aac-ld-mps-sbr-pcm',f'n512-ds2-x{x}-c2-base-crc1') for x in [1,2]]
    selections += [('aac-ld-mps-sbr-rates',f'n512-r48000-c2-x{x}') for x in [1,2]]
    for corpus,name in selections:
        parent=FIX/corpus
        source=next(c for c in json.loads((parent/'manifest.json').read_text())['controls'] if c['name']==name)
        data=(parent/'packets.bin').read_bytes()
        prefixes=[raw(data[r['offset']:r['offset']+r['bytes']])[:r['core_bits']] for r in source['frames'][:FRAMES]]
        # Each parent's own ASC writer places TERM+epConfig0 after its last
        # explicit downscale descriptor,or directly after SBR headers.
        wire=raw(bytes.fromhex(source['asc']))
        if source['factor']!=1:
            hint=source['rate']//(2 if source.get('double',False) else 1)
            ext=extension(hint,True);at=wire.rfind(ext+'000000');assert at>=0
            head=wire[:at]+ext
        else:
            from generate_aac_eld_downscale_info_fixtures import prefix
            head,_=prefix(source['asc'])
        assert packed(head+'000000').hex()==source['asc']
        stem=corpus+'-'+name
        base=dict(source,name=stem,frames=rows(prefixes),samples=FRAMES*source['out_n'],video=stem+'-control.mp4',golden=stem+'-control.f32le')
        (OUT/base['video']).write_bytes(video_fixture([base],blob,2,write=False)['data'])
        (OUT/base['golden']).write_bytes((parent/source['golden']).read_bytes()[:FRAMES*source['out_n']*2*4])
        controls.append(base)
        qmf=32 if source['rate']<27713 else 128 if source['rate']>=55426 else 64
        slots=source['out_n']//qmf;assert slots*qmf==source['out_n']
        def make_case(tree,bands,variant):
            model=next(c for c in models if c['tree']==tree and c['variant']==variant and c['bands']==bands)
            original=config(model,48000,slots,configs[model['config_offset']:model['config_offset']+model['config_bytes']],0)
            ssc_bits=frequency(source['rate'])+raw(original)[4:]
            assert ssc_bits[19]=='0'
            ssc_bits=ssc_bits[:19]+'1'+ssc_bits[20:]
            surround_index=(variant+tree)%5
            ssc_bits=ssc_bits[:20]+field(surround_index,3)+ssc_bits[23:34]+'1'+ssc_bits[35:52]
            for pattern in ['asymmetric','symmetric']:
                channels=[];hrtf_bits='00'+field({23:1,9:4,4:7}[bands],3)+field(pattern=='asymmetric',1)
                for ch in range(5):
                    left=[];right=[];phase=[]
                    for band in range(bands):
                        levels=[(5,1),(1,5),(3,4),(4,1),(1,4)]if band%3==0 else[(25,9),(9,25),(18,19),(21,7),(7,21)]
                        l,r=levels[ch];left.append(l+(band+ch)%2);right.append((r+(band+ch)%2)if pattern=='asymmetric'else left[-1]);phase.append((ch*3+band)%8)
                    hrtf_bits+=''.join(field(v,6)for v in left)+(''.join(field(v,6)for v in right)if pattern=='asymmetric'else'')+'1'+''.join(field(v,6)for v in phase)
                    channels.append(dict(left=left,right=right,phase=phase))
                ssc=packed(ssc_bits+hrtf_bits)
                asc=packed(head+field(2,4)+length(len(ssc))+raw(ssc)+'000000')
                frames,reference=[],[]
                for i,prefix in enumerate(prefixes):
                    meta=model['frames'][0]
                    bits='011'
                    for r in meta['parameters']:
                        offset,levels={'CLD':(15,31),'ICC':(0,8),'CPC':(20,51)}[r['kind']]
                        values=[r['values'][ps][b]+offset for b in range(len(r['values'][0])) for ps in range(2)]
                        bits+='1111'+'10001'+pcm(values,levels)
                    bits+='00'+'1001'
                    gains=[[[0 if i==0 else [-6,-3,0,3,6][(i+ps+band+2*ch)%5] for band in range(bands)] for ps in range(2)] for ch in range(2)]
                    for channel in gains:
                        bits+='1111'+'10001'+pcm([channel[ps][b]+15 for b in range(bands) for ps in range(2)],31)
                    body=packed(bits);reference.append(body.hex())
                    packet=prefix+envelope(9,1 if i==0 else 0,3,(((bytes([len(ssc)])if len(ssc)<127 else b'\x7f'+(len(ssc)-127).to_bytes(2,'big'))+ssc)if i==0 else b'')+body,True)
                    row=rows([packet])[0]
                    row.update(parameters=meta['parameters']+[dict(id=f'downmix-gain:{ch}',kind='CLD',start=0,values=values) for ch,values in enumerate(gains)],slots=[(slots+1)//2-1,slots-1],smoothing=[dict(time=256,mask=[False]*bands),dict(time=128,mask=[True]*bands)])
                    frames.append(row)
                case_name=f'{stem}-tree{tree}-b{bands}-v{variant}-{pattern}'
                case=dict(base,name=case_name,asc=asc.hex(),frames=frames,tree=tree,bands=bands,qmf_bins=qmf,channels=6 if tree==2 else 8,input_channels=2,group=1,slots=slots,ssc=ssc.hex(),reference=reference,core_gold=base['golden'],video=case_name+'.mp4',file=case_name+'.mkv',live_prefix_packets=3,edges=[source['out_n'],2*source['out_n'],3*source['out_n']],pattern=pattern,one_icc=model['one_icc'],ott_bands=model['ott_bands'],ttt=model['ttt'],inversion=model['inversion'],variant=variant,surround_index=surround_index,hrtf=channels,cutoffs=[0]*(len(model['ott_bands'])+1))
                (OUT/case['video']).write_bytes(video_fixture([case],blob,case['channels'],write=False)['data'])
                (OUT/case['file']).write_bytes(mkv(asc,[packed(raw(blob[r['offset']:r['offset']+r['bytes']])[:r['core_bits']]) for r in frames],source['rate'],source['out_n'],case['channels']))
                cases.append(case)
        for tree in [2,3,4]:
            for bands in [23,9,4]:
                for variant in range(6):
                    make_case(tree,bands,variant)
    # Valid syntax, deliberately unequal parameter/HRTF grids. Both ASC and
    # first in-band SSC carry the mismatch, isolating the unsupported mapping.
    base=cases[0]; hb='00'+field(4,3)+'1'
    for channel in base['hrtf']:
        hb+=''.join(field(v,6)for v in channel['left'][:9])+''.join(field(v,6)for v in channel['right'][:9])+'1'+''.join(field(v,6)for v in channel['phase'][:9])
    old=bytes.fromhex(base['ssc']); ssc=packed(raw(old)[:52]+hb)
    wire=raw(bytes.fromhex(base['asc']));marker=field(2,4)+length(len(old))+raw(old)+'000000'
    assert wire.startswith(wire[:wire.rfind(marker)]+marker)
    asc=packed(wire[:wire.rfind(marker)]+field(2,4)+length(len(ssc))+raw(ssc)+'000000')
    c=controls[0]; row=c['frames'][0]; prefix=raw(blob[row['offset']:row['offset']+row['bytes']])[:row['core_bits']]
    packet=prefix+envelope(9,1,3,bytes([len(ssc)])+ssc+bytes.fromhex(base['reference'][0]),True)
    bad=dict(base,name='unequal-hrtf-grid',asc=asc.hex(),ssc=ssc.hex(),frames=rows([packet]),samples=base['out_n'],video='unequal-hrtf-grid.mp4',error='HRTF grid conversion is not yet qualified')
    (OUT/bad['video']).write_bytes(video_fixture([bad],blob,bad['channels'],write=False)['data'])
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,errors=[bad],exports=[dict(c,video=c['file'],name=c['name']+'-matroska') for c in cases],scope='Own signalled3D-stereo HRTF-set0,TTT trees2/3/4,all six modes,23/9/4 equal grids,asymmetric/symmetric responses,standard inverse priority and determinant-floor regularization. Own parameters,no KEMAR/private data. Plain ELD and LD-SBR PCM paths,32/64/128-band spatial banks. Independent saved input PCM plus qualified spatial composition;not full arbitrary-downmix/all-tree conformance.'),separators=(',',':'))+'\n')
    print(len(cases),'programmes;',len(controls),'independent input controls')
if __name__=='__main__':main()
