#!/usr/bin/env python3
"""Own stereo arbitrary-downmix TTT gain movies; explicit offline generation."""
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
OUT=FIX/'aac-ld-mps-arbdmx-ttt'
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
            ssc=packed(ssc_bits[:19]+'1'+ssc_bits[20:])
            asc=packed(head+field(2,4)+length(len(ssc))+raw(ssc)+'000000')
            for pattern in ['alternating']:
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
                    packet=prefix+envelope(9,1 if i==0 else 0,3,(bytes([len(ssc)])+ssc if i==0 else b'')+body,True)
                    row=rows([packet])[0]
                    row.update(parameters=meta['parameters']+[dict(id=f'downmix-gain:{ch}',kind='CLD',start=0,values=values) for ch,values in enumerate(gains)],slots=[(slots+1)//2-1,slots-1],smoothing=[dict(time=256,mask=[False]*bands),dict(time=128,mask=[True]*bands)])
                    frames.append(row)
                case_name=f'{stem}-tree{tree}-b{bands}-v{variant}-{pattern}'
                case=dict(base,name=case_name,asc=asc.hex(),frames=frames,tree=tree,bands=bands,qmf_bins=qmf,channels=6 if tree==2 else 8,input_channels=2,group=1,slots=slots,ssc=ssc.hex(),reference=reference,core_gold=base['golden'],video=case_name+'.mp4',file=case_name+'.mkv',live_prefix_packets=3,edges=[source['out_n'],2*source['out_n'],3*source['out_n']],pattern=pattern,one_icc=model['one_icc'],ott_bands=model['ott_bands'],ttt=model['ttt'],inversion=model['inversion'],variant=variant,cutoffs=[0]*(len(model['ott_bands'])+1))
                (OUT/case['video']).write_bytes(video_fixture([case],blob,case['channels'],write=False)['data'])
                (OUT/case['file']).write_bytes(mkv(asc,[packed(raw(blob[r['offset']:r['offset']+r['bytes']])[:r['core_bits']]) for r in frames],source['rate'],source['out_n'],case['channels']))
                cases.append(case)
        for tree in [2,3,4]:
            for bands in [23,9,4]:
                for variant in range(6):
                    make_case(tree,bands,variant)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,exports=[dict(c,video=c['file'],name=c['name']+'-matroska') for c in cases],scope='Own stereo arbitrary-downmix,TTT trees2/3/4,all six mode combinations,23/9/4 bands,initial unity then unequal time-frequency varying gains before inversion. Plain ELD and LD-SBR PCM paths,32/64/128-band spatial banks. Independent saved input PCM plus qualified spatial composition;not full arbitrary-downmix/all-tree conformance.'),separators=(',',':'))+'\n')
    print(len(cases),'programmes;',len(controls),'independent input controls')
if __name__=='__main__':main()
