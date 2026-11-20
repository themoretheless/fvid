#!/usr/bin/env python3
"""Explicit offline scalar-oracle qualification of combined 22.2 ELD tools."""
import copy, json, struct
from pathlib import Path
from generate_aac_22_2_fixtures import ELEMENTS, MAP
from generate_aac_ld_layout_fixtures import LAYOUTS
from generate_aac_eld_fixtures import generate as eld
from generate_aac_eld_sbr_fixtures import analysis, reconstruct, sbr_bits
from generate_aac_eld_cldfb_fixtures import window, synthesis
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_sbr_frequency_oracles import tables
from generate_he_aac_packet_fixtures import field, frequency, packed, video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-22-2-sbr-resilience'
def main():
    OUT.mkdir(exist_ok=True); LAYOUTS[13]=(ELEMENTS,MAP,0)
    noise_data=(ROOT/'tests/fixtures/playback-errors/aac-sbr-noise-protocol.f64le').read_bytes()
    noise=[complex(*struct.unpack_from('<dd',noise_data,i*16))for i in range(512)]
    coeffs=window();_,hi,lo,_=tables(10,27,0,False,0,0)
    units=bytearray();plain=bytearray();cases=[]
    specs=[(n,flags,'resilience',True,True)for n in [480,512]for flags in [2,3,6,7]]
    specs += [(n,7,'tns',True,True)for n in [480,512]]
    specs += [(n,7,'resilience',False,False)for n in [480,512]]
    for n,flags,tool,double,crc in specs:
        source=eld(True,[(n,24000,13,tool,flags,0,False)],include_category_bits=True,frame_count=4)
        base=json.loads(source['manifest.json'])['cases'][0]
        name=f'n{n}-flags{flags}-{tool}-x{2 if double else 1}-crc{int(crc)}'
        core=source[base['name']+'.f32le'];original=struct.unpack('<'+'f'*(len(core)//4),core)
        rows=[([r+[0j]*32 for r in analysis(original[ch::24],coeffs,32)]if ch in [10,11]
               else reconstruct(analysis(original[ch::24],coeffs),n//32,noise))for ch in range(24)]
        planar=[[v/(32768 if double else 65536)for v in synthesis(r,64 if double else 32,coeffs)]for r in rows]
        pcm=b''.join(struct.pack('<24f',*(ch[i]for ch in planar))for i in range(len(planar[0])))
        prefix=field(31,5)+field(7,6)+frequency(24000)+field(13,4)+field(n==480,1)+field(flags,3)+'1'+field(double,1)+field(crc,1)+header(0,True)*14+'0000'
        asc=packed(prefix+'01');control_asc=packed(prefix+'00');frames=[];ep1_blob=bytearray();ep1_frames=[]
        for i,r in enumerate(base['frames']):
            parts=copy.deepcopy(r['esc'])
            for kind in ELEMENTS:
                if kind!=3:parts+=sbr_bits(2 if kind==1 else 1,len(hi)-1,len(lo)-1,n//32,i,crc,categories=True)
            count=len(parts);parts[-1]['bits']+='0'*(-sum(len(p['bits'])for p in parts)%8)
            groups=[]
            for ordinal,start in enumerate(range(0,count,2)):
                wire=''.join(p['bits']for p in parts[start:start+2]);raw=bytearray(packed(wire))
                if len(wire)%8:raw[-1]|=(1<<(8-len(wire)%8))-1
                groups.append(dict(offset=len(units),bytes=len(raw),bits=len(wire),stream_id=100+ordinal*3,depends_on=None if ordinal==0 else 100+(ordinal-1)*3));units.extend(raw)
            # Flat transport follows ordinary syntax, not ESC category order.
            wire=r['bits']+''.join(sbr_bits(2 if kind==1 else 1,len(hi)-1,len(lo)-1,n//32,i,crc)for kind in ELEMENTS if kind!=3)
            category_data=packed(''.join(p['bits']for p in parts))
            ep1_frames.append(dict(offset=len(ep1_blob),bytes=len(category_data)));ep1_blob.extend(category_data)
            data=packed(wire);frames.append(dict(offset=len(plain),bytes=len(data),units=groups[::-1]));plain.extend(data)
        hop=n*(2 if double else 1);rate=24000*(2 if double else 1)
        c=dict(name=name,n=hop,rate=rate,channels=24,configuration=13,flags=flags,tool=tool,asc=asc.hex(),plain_asc=control_asc.hex(),instances=count,groups=len(groups),frames=frames,container_rate=rate,container_frame_samples=hop,samples=4*hop,pcm_offset=0,slots=n//32,bands=64 if double else 32)
        movie=video_fixture([dict(c,asc=control_asc.hex())],plain,24,write=False)['data']
        (OUT/(name+'-control.mp4')).write_bytes(movie);(OUT/(name+'.mp4')).write_bytes(video_fixture([dict(c,frames=ep1_frames)],ep1_blob,24,write=False)['data']);(OUT/(name+'.f32le')).write_bytes(pcm);cases.append(c)
    (OUT/'units.bin').write_bytes(units);(OUT/'plain.bin').write_bytes(plain)
    (OUT/'manifest.json').write_text(json.dumps(dict(ep1=cases,scope='Twelve authored 24-channel LD-SBR programmes, RVLC/HCR/section resilience and TNS combinations. Four AUs each. Independent scalar core and CLDFB/HF/synthesis oracle. Grouped EP1 typed API and single-track MP4; no multi-track or Matroska claim. Explicit generation, no FFmpeg/network/private media.'),indent=2)+'\n')
    print('Generated',len(cases),'programmes')
if __name__=='__main__':main()
