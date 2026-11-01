#!/usr/bin/env python3
"""Authored24-channel ELD EP3/EP1 movies and scalar PCM; offline."""
import copy, json
from pathlib import Path
from generate_aac_22_2_fixtures import ELEMENTS, MAP, movie_mkv
from generate_aac_ld_layout_fixtures import LAYOUTS
from generate_aac_eld_fixtures import generate as eld
from generate_aac_eld_esc_fixtures import configuration, protect
from generate_he_aac_packet_fixtures import video_fixture, packed
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-22-2-esc'
def main():
    OUT.mkdir(exist_ok=True);LAYOUTS[13]=(ELEMENTS,MAP,0)
    source=eld(True,[(n,48000,13,'base' if flags==0 else 'resilience',flags,0,False)for n in [480,512]
        for flags in [0,1,4,5]],include_category_bits=True,frame_count=4)
    blob,plain,cases=bytearray(),bytearray(),[]
    for base in json.loads(source['manifest.json'])['cases']:
        patterns=list(dict.fromkeys(tuple(p['category']for p in row['esc'])for row in base['frames']))
        assert all(len(pattern)==56 for pattern in patterns)
        orders=[list(reversed(range(len(pattern))))for pattern in patterns]
        asc,_,_,_=configuration(base,patterns,orders)
        name=f"n{base['n']}-flags{base['flags']}";frames=[];packets=[];bad=[]
        for i,old in enumerate(base['frames']):
            row=copy.deepcopy(old);row['esc'][-1]['bits']+='0'*(-sum(len(p['bits'])for p in row['esc'])%8)
            encoded,_=protect(row,patterns,orders)
            damaged,_=protect(row,patterns,orders,'crc'if i==2 else None)
            control=source['packets.bin'][old['offset']:old['offset']+old['bytes']]
            frames.append(dict(offset=len(blob),bytes=len(encoded),plain_offset=len(plain),plain_bytes=len(control)))
            blob+=encoded;plain+=control;packets.append(encoded);bad.append(damaged)
        c=dict(name=name,n=base['n'],rate=48000,channels=24,configuration=13,
            flags=base['flags'],asc=asc.hex(),plain_asc=base['asc'],classes=56,frames=frames,
            container_rate=48000,container_frame_samples=base['n'],samples=4*base['n'],
            pcm_offset=0,slots=base['n']//64,bands=32)
        (OUT/(name+'.mp4')).write_bytes(video_fixture([c],blob,24,write=False)['data'])
        (OUT/(name+'.mkv')).write_bytes(movie_mkv(asc,packets,base['n']))
        # A saved malformed movie reaches the EP class CRC, not a container error.
        damaged_blob=bytearray();rows=[]
        for packet in bad:rows.append(dict(offset=len(damaged_blob),bytes=len(packet)));damaged_blob+=packet
        (OUT/(name+'-crc.mp4')).write_bytes(video_fixture([dict(c,frames=rows)],damaged_blob,24,write=False)['data'])
        (OUT/(name+'.f32le')).write_bytes(source[base['name']+'.f32le']);cases.append(c)
    # RVLC adds a third per-channel category (80 instances per frame).
    # EP1 transports arbitrary consecutive instance groups without inventing
    # an illegal >63-class EP3 configuration.
    units=bytearray();ep1=[]
    source=eld(True,[(n,48000,13,'resilience',flags,0,False)for n in [480,512]
        for flags in [2,3,6,7]],include_category_bits=True,frame_count=4)
    for base in json.loads(source['manifest.json'])['cases']:
        name=f"ep1-n{base['n']}-flags{base['flags']}";rows=[]
        bits=''.join(f'{b:08b}'for b in bytes.fromhex(base['asc']))
        assert bits[28:30]=='00';asc=packed(bits[:28]+'01')
        for i,old in enumerate(base['frames']):
            parts=copy.deepcopy(old['esc']);assert len(parts)==80
            parts[-1]['bits']+='0'*(-sum(len(p['bits'])for p in parts)%8)
            groups=[]
            for ordinal,start in enumerate(range(0,len(parts),2)):
                wire=''.join(p['bits']for p in parts[start:start+2]);raw=bytearray(packed(wire))
                if len(wire)%8:raw[-1]|=(1<<(8-len(wire)%8))-1
                groups.append(dict(offset=len(units),bytes=len(raw),bits=len(wire),stream_id=100+ordinal*3,
                    depends_on=None if ordinal==0 else 100+(ordinal-1)*3))
                units.extend(raw)
            original=source['packets.bin'][old['offset']:old['offset']+old['bytes']]
            rows.append(dict(offset=len(plain),bytes=len(original),units=groups[::-1]))
            plain.extend(original)
        c=dict(name=name,n=base['n'],rate=48000,channels=24,configuration=13,
            flags=base['flags'],asc=asc.hex(),plain_asc=base['asc'],instances=80,groups=40,frames=rows,
            container_rate=48000,container_frame_samples=base['n'],samples=4*base['n'],pcm_offset=0,
            slots=base['n']//64,bands=32)
        control=video_fixture([dict(c,asc=base['asc'])],plain,24,write=False)['data']
        from generate_mp4_ep1_grouped_ps_fixtures import replace_asc
        (OUT/(name+'-control.mp4')).write_bytes(control)
        (OUT/(name+'.mp4')).write_bytes(replace_asc(control,asc))
        (OUT/(name+'.f32le')).write_bytes(source[base['name']+'.f32le']);ep1.append(c)
    from generate_aac_eld_sbr_fixtures import sbr_bits
    from generate_aac_sbr_dsp_fixtures import header as sbr_header
    from generate_aac_sbr_frequency_oracles import tables
    from generate_he_aac_packet_fixtures import field, frequency
    parent=ROOT/'tests/fixtures/playback-errors/aac-22-2'
    parent_manifest=json.loads((parent/'manifest.json').read_text())
    parent_packets=(parent/'packets.bin').read_bytes();gold=(parent/'reference.f32le').read_bytes()
    source=eld(True,[(n,24000,13,'base',0,0,False)for n in [480,512]],include_category_bits=True,frame_count=4)
    _,hi,lo,_=tables(10,27,0,False,0,0)
    for base in json.loads(source['manifest.json'])['cases']:
        n=base['n'];name=f'ep1-sbr-n{n}-crc1';rows=[]
        old=next(c for c in parent_manifest['cases']if c['name']==f'aot39-sbr-n{n}-crc1')
        prefix=field(31,5)+field(7,6)+frequency(24000)+field(13,4)+field(n==480,1)+'000111'+sbr_header(0,True)*14+'0000'
        assert packed(prefix+'00').hex()==old['asc'];asc=packed(prefix+'01')
        for i,r in enumerate(base['frames']):
            parts=copy.deepcopy(r['esc'])
            for kind in ELEMENTS:
                if kind!=3:parts+=sbr_bits(2 if kind==1 else 1,len(hi)-1,len(lo)-1,n//32,i,True,categories=True)
            assert len(parts)==84
            parts[-1]['bits']+='0'*(-sum(len(p['bits'])for p in parts)%8)
            groups=[]
            for ordinal,start in enumerate(range(0,len(parts),2)):
                wire=''.join(p['bits']for p in parts[start:start+2]);raw=bytearray(packed(wire))
                if len(wire)%8:raw[-1]|=(1<<(8-len(wire)%8))-1
                groups.append(dict(offset=len(units),bytes=len(raw),bits=len(wire),stream_id=100+ordinal*3,
                    depends_on=None if ordinal==0 else 100+(ordinal-1)*3));units.extend(raw)
            original=packed(''.join(p['bits']for p in parts));r0=old['frames'][i]
            assert original==parent_packets[r0['offset']:r0['offset']+r0['bytes']]
            rows.append(dict(offset=len(plain),bytes=len(original),units=groups[::-1]));plain.extend(original)
        c=dict(name=name,n=2*n,rate=48000,channels=24,configuration=13,flags=0,
            asc=asc.hex(),plain_asc=old['asc'],instances=84,groups=42,frames=rows,
            container_rate=48000,container_frame_samples=2*n,samples=8*n,pcm_offset=0,slots=n//32,bands=64)
        control=video_fixture([dict(c,asc=old['asc'])],plain,24,write=False)['data']
        (OUT/(name+'-control.mp4')).write_bytes(control)
        (OUT/(name+'.mp4')).write_bytes(replace_asc(control,asc))
        at=old['reference_offset'];(OUT/(name+'.f32le')).write_bytes(gold[at:at+8*n*24*4]);ep1.append(c)
    (OUT/'units.bin').write_bytes(units)
    (OUT/'packets.bin').write_bytes(blob);(OUT/'plain.bin').write_bytes(plain)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,ep1=ep1,scope='Eight authored24-channel ELD EP3 programmes with56 independently protected/reordered ESC classes per AU; bare, HCR, section-resilience and both. Separate scalar ELD oracle. Four AUs each; original valid and specifically CRC-damaged short videos. No private media, foreign decoder, FFmpeg, subprocess or network. Eight further RVLC/resilience EP1 programmes carry80 instances in40 dependency-ordered groups, plus one-AU MP4 grouped transport controls. Two further LD-SBR/CRC EP1 cases carry84 instances in42 groups with both LFEs. No >63-class EP3 configuration is invented.'),indent=2)+'\n')
    print('Authored22.2 protected:',len(cases),'EP3 and',len(ep1),'EP1 programmes')
if __name__=='__main__':main()
