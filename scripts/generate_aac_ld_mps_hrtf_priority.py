#!/usr/bin/env python3
"""Own HRTF metadata bypass with qualified standard-inversion priority."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,packed,video_fixture
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-hrtf-priority'
def main():
    OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-hrtf-all-grids';m=json.loads((parent/'manifest.json').read_text());source=(parent/'packets.bin').read_bytes();blob=bytearray();controls=[];cases=[];errors=[]
    by_pair={(c['spatial_bands'],c['hrtf_bands']):c for c in m['cases']+m['errors']}
    def ssc(c,priority=True,hrtf=True):
        b=raw(bytes.fromhex(c['ssc']));b=b[:29]+field(priority,1)+b[30:]
        if not hrtf:b=b[:34]+'0'+b[35:52]
        return packed(b)
    def asc(c,data):
        old=bytes.fromhex(c['ssc']);bits=raw(bytes.fromhex(c['asc']));marker=field(2,4)+length(len(old))+raw(old)+'000000';at=bits.rfind(marker);assert at>=0
        return packed(bits[:at]+field(2,4)+length(len(data))+raw(data)+'000000')
    def save(c,name,configs,all_headers=False,control_name=None):
        frames=[];packets=[]
        for i,r in enumerate(c['frames']):
            old=raw(source[r['offset']:r['offset']+r['bytes']]);prefix=old[:r['prefix_bits']];body=bytes.fromhex(c['reference'][i]);head=i==0 or all_headers
            packet=packed(prefix+envelope(9,1 if head else 0,3,header(configs[i],body)if head else body,True));frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
        a=asc(c,configs[0]);case=dict(c,name=name,frames=frames,asc=a.hex(),ssc=configs[0].hex(),configs=[v.hex()for v in configs],video=name+'.mp4',file=name+'.mkv',inversion=True,acceptance=True,control_name=control_name)
        case.pop('golden',None);case.pop('core_gold',None)
        (OUT/case['video']).write_bytes(video_fixture([case],blob,6,write=False)['data']);(OUT/case['file']).write_bytes(mkv(a,packets,c['rate'],c['out_n'],6));return case
    for bands in [23,15,12,9,7,5,4]:
        c=by_pair[bands,bands];plain=ssc(c,hrtf=False);control_name=f'spatial{bands}-standard-control'
        controls.append(save(c,control_name,[plain]*3))
        for hb in [23,15,12,9,7,5,4]:
            c=by_pair[bands,hb];data=ssc(c);cases.append(save(c,f'spatial{bands}-hrtf{hb}-priority',[data]*3,control_name=control_name))
    c=by_pair[9,23];plain=ssc(c,hrtf=False);controls.append(save(c,'cycle-standard-control',[plain]*3,True))
    configs=[ssc(by_pair[9,h])for h in [23,4,15]]
    cases.append(save(c,'spatial9-metadata-cycle',configs,True,'cycle-standard-control'))
    c=by_pair[9,4];priority=ssc(c);active=ssc(c,priority=False)
    errors.append(save(c,'spatial9-activate-unqualified-hrtf',[priority,active,priority],True,'spatial9-standard-control'));errors[-1]['acceptance']=False
    (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(controls=controls,cases=cases,errors=errors,scope='All49 grid pairs with HRTF unused because standard inversion has priority in both active TTT regions. Three AUs; seven no-HRTF controls, one repeated-header control, one metadata-cycle programme and one negative activation movie. These are equivalence tests, not general unequal-grid HRTF processing. Own authored inputs; no private/ISO media, FFmpeg, process or network.'),separators=(',',':'))+'\n');print('Generated',len(cases),'priority programmes,',len(controls),'controls,',len(errors),'activation refusal')
if __name__=='__main__':main()
