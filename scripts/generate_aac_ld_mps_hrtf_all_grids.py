#!/usr/bin/env python3
"""Offline all49 grid-pair reproducers, with three-AU equal-grid controls."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_mps_stp_fixtures import config
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-hrtf-all-grids';GRIDS=[23,15,12,9,7,5,4]
def main():
    OUT.mkdir(exist_ok=True)
    parent=FIX/'aac-ld-mps-3d';old=json.loads((parent/'manifest.json').read_text());source=(parent/'packets.bin').read_bytes()
    base=old['controls'][0];original=next(c for c in old['cases']if c['tree']==2 and c['variant']==0 and c['bands']==23 and c['pattern']=='asymmetric'and c['core_gold']==base['golden'])
    old_ssc=bytes.fromhex(original['ssc']);asc_bits=raw(bytes.fromhex(original['asc']));marker=field(2,4)+length(len(old_ssc))+raw(old_ssc)+'000000';start=asc_bits.rfind(marker);assert start>=0
    head=asc_bits[:start];blob=bytearray();controls=[];cases=[];errors=[]
    def save_rows(packets):
        rows=[]
        for p in packets:rows.append(dict(offset=len(blob),bytes=len(p)));blob.extend(p)
        return rows
    prefixes=[raw(source[r['offset']:r['offset']+r['bytes']])[:r['core_bits']]for r in base['frames'][:3]]
    core_packets=[packed(p)for p in prefixes]
    control=dict(base,name='core-control',frames=save_rows(core_packets),samples=3*base['out_n'],video='core-control.mp4',golden='core-control.f32le')
    (OUT/control['video']).write_bytes(video_fixture([control],blob,2,write=False)['data'])
    (OUT/control['golden']).write_bytes((parent/base['golden']).read_bytes()[:3*base['out_n']*2*4]);controls.append(control)
    models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];configs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes()
    for bands in GRIDS:
        model=next(c for c in models if c['tree']==2 and c['variant']==0 and c['bands']==bands)
        raw_cfg=config(model,48000,base['slots'],configs[model['config_offset']:model['config_offset']+model['config_bytes']],0)
        bits=frequency(base['rate'])+raw(raw_cfg)[4:];bits=bits[:19]+'1'+bits[20:]
        bits=bits[:20]+field(2,3)+bits[23:34]+'1'+bits[35:52]
        references=[];frame_meta=[]
        for i in range(3):
            meta=model['frames'][0];wire='011'
            for r in meta['parameters']:
                offset,levels={'CLD':(15,31),'ICC':(0,8),'CPC':(20,51)}[r['kind']]
                wire+='1111'+'10001'+pcm([r['values'][ps][b]+offset for b in range(len(r['values'][0]))for ps in range(2)],levels)
            wire+='00'+'1001'
            gains=[[[0 if i==0 else[-6,-3,0,3,6][(i+ps+b+2*ch)%5]for b in range(bands)]for ps in range(2)]for ch in range(2)]
            for channel in gains:wire+='1111'+'10001'+pcm([channel[ps][b]+15 for b in range(bands)for ps in range(2)],31)
            references.append(packed(wire).hex())
            frame_meta.append(dict(parameters=meta['parameters']+[dict(id=f'downmix-gain:{ch}',kind='CLD',start=0,values=v)for ch,v in enumerate(gains)],slots=[(base['slots']+1)//2-1,base['slots']-1],smoothing=[dict(time=256,mask=[False]*bands),dict(time=128,mask=[True]*bands)]))
        for hbands in GRIDS:
            h='00'+field(GRIDS.index(hbands)+1,3)+'1';hrtf=[]
            for ch in range(5):
                left=[];right=[];phase=[]
                for b in range(hbands):
                    levels=[(5,1),(1,5),(3,4),(4,1),(1,4)]if b%3==0 else[(25,9),(9,25),(18,19),(21,7),(7,21)]
                    l,r=levels[ch];left.append(l+(b+ch)%2);right.append(r+(b+ch)%2);phase.append((ch*3+b)%8)
                h+=''.join(field(v,6)for v in left+right)+'1'+''.join(field(v,6)for v in phase);hrtf.append(dict(left=left,right=right,phase=phase))
            ssc=packed(bits+h);asc=packed(head+field(2,4)+length(len(ssc))+raw(ssc)+'000000')
            packets=[packed(prefix+envelope(9,1 if i==0 else 0,3,header(ssc,bytes.fromhex(references[i]))if i==0 else bytes.fromhex(references[i]),True))for i,prefix in enumerate(prefixes)]
            rows=save_rows(packets)
            for row,meta,prefix in zip(rows,frame_meta,prefixes):row.update(meta,prefix_bits=len(prefix))
            name=f'spatial{bands}-hrtf{hbands}'
            c=dict(base,name=name,frames=rows,asc=asc.hex(),ssc=ssc.hex(),reference=references,hrtf=hrtf,tree=2,variant=0,pattern='asymmetric',bands=bands,spatial_bands=bands,hrtf_bands=hbands,channels=6,input_channels=2,group=1,core_gold=control['golden'],samples=3*base['out_n'],video=name+'.mp4',file=name+'.mkv',one_icc=model['one_icc'],ott_bands=model['ott_bands'],ttt=model['ttt'],inversion=model['inversion'],surround_index=2,cutoffs=[0]*4,acceptance=bands==hbands)
            if bands==hbands and bands in [23,9,4]:
                prior=next(c for c in old['cases']if c['tree']==2 and c['variant']==0 and c['bands']==bands and c['pattern']=='asymmetric'and c['core_gold']==base['golden'])
                assert prior['ssc']==ssc.hex() and prior['reference'][:3]==references
                assert all(p==source[r['offset']:r['offset']+r['bytes']]for p,r in zip(packets,prior['frames']))
                c['original_control']=prior['video']
            if bands!=hbands:c['error']='HRTF grid conversion is not yet qualified'
            (OUT/c['video']).write_bytes(video_fixture([c],blob,6,write=False)['data']);(OUT/c['file']).write_bytes(mkv(asc,packets,c['rate'],c['out_n'],6))
            (cases if bands==hbands else errors).append(c)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(controls=controls,cases=cases,errors=errors,scope='All49 spatial/HRTF grid pairs, three authored AUs each, MP4 and Matroska. Seven equal-grid integration controls;42 specific unequal-grid refusal reproducers, NOT unequal-grid acceptance. Plain ELD downscaled to24kHz,32-bin bank,tree525/modes0+1. Independent saved core PCM plus owned spatial component comparison; no new independent whole-spatial oracle. No private/ISO media, FFmpeg, process or network.'),separators=(',',':'))+'\n')
    print('Generated',len(cases),'equal controls and',len(errors),'unequal refusals')
if __name__=='__main__':main()
