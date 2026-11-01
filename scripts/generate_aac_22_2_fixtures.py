#!/usr/bin/env python3
"""Authored indexed AAC22.2 movies and sparse scalar PCM; explicit/offline."""
from pathlib import Path
import json, math, struct
from generate_he_aac_packet_fixtures import field, frequency, packed, video_fixture
from generate_aac_main_tools_fixtures import tuple_bits, sc, f32
from generate_aac_eld_fixtures import generate as eld
from generate_aac_ld_layout_fixtures import LAYOUTS, body as ld_body, Oracle, geometry
from generate_aac_device_layout_fixtures import ebml, picture
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT/'tests/fixtures/playback-errors/aac-22-2'
ELEMENTS = [0,1,1,1,1,0,3,3,0,1,1,0,1,0,0,1]
# Explicit normal/top/bottom speaker order, not a fictitious WAVE mask.
MAP = [0,1,2,3,4,5,6,8,9,7,10,11,12,13,14,16,17,15,19,20,18,21,22,23]
# Emitted roles: normal FC/FLC/FRC/FL/FR/SL/SR/BC/BL/BR/LFE1/LFE2,
# top FC/FL/FR/C/SL/SR/BC/BL/BR, bottom FC/FL/FR.
GROUPS = [('Normal','Front',5),('Normal','Side',2),('Normal','Back',3),
          ('Normal','Lfe',2),('Top','Front',3),('Top','Side',3),
          ('Top','Back',3),('Bottom','Front',3)]

def movie_mkv(asc, packets, n):
    video = ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+
        ebml('86',b'V_VP9')+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
    audio = ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+
        ebml('86',b'A_AAC')+ebml('63a2',asc)+ebml('e1',
        ebml('b5',struct.pack('>d',48000))+ebml('9f',b'\x18')))
    clusters = b''.join(ebml('1f43b675',ebml('e7',(i*n*1000000000//48000).to_bytes(8,'big'))+
        ebml('a3',b'\x81\0\0\x80'+picture(8,'420',16,16))+
        ebml('a3',b'\x82\0\0\x80'+p)) for i,p in enumerate(packets))
    return ebml('1a45dfa3',ebml('4282',b'matroska'))+ebml('18538067',
        ebml('1549a966',ebml('2ad7b1',b'\1'))+ebml('1654ae6b',video+audio)+clusters)

def generate():
    OUT.mkdir(exist_ok=True)
    blob, gold, cases = bytearray(), bytearray(), []
    def save(name, asc, packets, pcm, n, aot):
        frames=[]
        for packet in packets:
            frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet)
        c=dict(name=name,asc=asc.hex(),n=n,rate=48000,channels=24,object_type=aot,
            container_rate=48000,container_frame_samples=n,samples=len(packets)*n,
            pcm_offset=0,slots=n//64,bands=64 if n>512 else 32,frames=frames,reference_offset=len(gold),reference_bytes=len(pcm))
        gold.extend(pcm)
        (OUT/(name+'.mp4')).write_bytes(video_fixture([c],blob,24,write=False)['data'])
        (OUT/(name+'.mkv')).write_bytes(movie_mkv(asc,packets,n))
        cases.append(c)
    # Main/LC/LTP and ER forms: no prediction is signalled. Each channel has
    # a distinct scale/time pattern, with independent direct cosine+overlap.
    for aot,n in [(1,1024),(2,1024),(2,960),(4,1024),(17,1024),(19,960)]:
        asc=packed(field(aot,5)+frequency(48000)+field(13,4)+field(n==960,1)+'00'+('00'if aot in [17,19]else''))
        overlap=[[0.]*n for _ in MAP];packets=[];pcm=bytearray()
        cos=[[math.cos(math.pi/n*(t+.5+n/2)*(k+.5))for k in range(4)]for t in range(2*n)]
        for frame in range(6):
            wire='';ch=0;tags=[0]*4;signals=[]
            for kind in ELEMENTS:
                if aot not in [17,19]:wire+=field(kind,3)
                wire+=field(tags[kind],4);tags[kind]+=1
                if kind==1:wire+='0'
                for _ in range(2 if kind==1 else 1):
                    gain=96+3*ch;sign=1 if (frame+ch)%3 else -1
                    wire+=field(gain,8)+'0000'+field(1,6)+'0'+field(1,4)+field(1,5)+sc(0)+'000'+tuple_bits([sign]*4)
                    amplitude=f32(sign*2**((gain-100)/4))
                    block=[sum(amplitude*c for c in row)*2/n/65536*math.sin(math.pi/(2*n)*(t+.5))for t,row in enumerate(cos)]
                    signals.append([block[i]+overlap[ch][i]for i in range(n)])
                    overlap[ch]=block[n:];ch+=1
            wire+='111'if aot not in [17,19]else''
            packets.append(packed(wire))
            for i in range(n):
                row=[0.]*24
                for ch,target in enumerate(MAP):row[target]=signals[ch][i]
                pcm.extend(struct.pack('<24f',*row))
        save(f'aot{aot}-n{n}',asc,packets,pcm,n,aot)
    for n in [480,512]:
        count,high=geometry(n,48000)
        bins=list(range(4))+list(range(high,high+4))
        cos=[[math.cos(math.pi/n*(i+.5+n/2)*(k+.5))for i in range(2*n)]for k in bins]
        banks=[Oracle(n,high,cos)for _ in MAP];pcm=bytearray();packets=[]
        for frame in range(6):
            wire='';signals=[];ch=0
            for kind in ELEMENTS:
                wire+='0000'+('0'if kind==1 else'')
                for _ in range(2 if kind==1 else 1):
                    bits,params=ld_body(n,count,high,frame,ch,kind==3)
                    wire+=bits;signals.append(banks[ch].run(*params));ch+=1
            packets.append(packed(wire))
            for i in range(n):
                row=[0.]*24
                for ch,target in enumerate(MAP):row[target]=signals[ch][i]
                pcm.extend(struct.pack('<24f',*row))
        asc=packed(field(23,5)+frequency(48000)+field(13,4)+field(n==480,1)+'0000')
        save(f'aot23-n{n}',asc,packets,pcm,n,23)
    # The existing separate ELD oracle performs direct 4N cosines/window/overlap.
    LAYOUTS[13]=(ELEMENTS,MAP,0)
    for n in [480,512]:
        files=eld(specs=[(n,48000,13,'base',0,0,False)],frame_count=6)
        c=json.loads(files['manifest.json'])['cases'][0];data=files['packets.bin']
        packets=[data[r['offset']:r['offset']+r['bytes']]for r in c['frames']]
        save(f'aot39-n{n}',bytes.fromhex(c['asc']),packets,files[c['name']+'.f32le'],n,39)
    from generate_aac_eld_sbr_fixtures import analysis, reconstruct, sbr_bits
    from generate_aac_eld_cldfb_fixtures import window, synthesis
    from generate_aac_sbr_dsp_fixtures import header
    from generate_aac_sbr_frequency_oracles import tables
    noise_data=(ROOT/'tests/fixtures/playback-errors/aac-sbr-noise-protocol.f64le').read_bytes()
    noise=[complex(*struct.unpack_from('<dd',noise_data,i*16))for i in range(512)]
    coeffs=window();_,high,low,_=tables(10,27,0,False,0,0)
    for n in [480,512]:
        files=eld(True,[(n,24000,13,'base',0,0,False)],frame_count=6)
        base=json.loads(files['manifest.json'])['cases'][0]
        original=list(struct.unpack('<'+'f'*(len(files[base['name']+'.f32le'])//4),files[base['name']+'.f32le']))
        # Two independent LFE channels bypass HF reconstruction, not just the
        # first LFE in the indexed sequence. All other roles use LD-SBR.
        rows=[([r+[0j]*32 for r in analysis(original[ch::24],coeffs,32)]
               if ch in [10,11] else reconstruct(analysis(original[ch::24],coeffs),n//32,noise))
              for ch in range(24)]
        planar=[[v/32768 for v in synthesis(r,64,coeffs)]for r in rows]
        pcm=b''.join(struct.pack('<24f',*(ch[i]for ch in planar))for i in range(len(planar[0])))
        for protected in [False,True]:
            asc=packed(field(31,5)+field(7,6)+frequency(24000)+field(13,4)+field(n==480,1)+
                '000'+'1'+'1'+field(protected,1)+header(0,True)*14+'0000'+'00')
            packets=[packed(r['bits']+''.join(sbr_bits(2 if e==1 else 1,len(high)-1,len(low)-1,
                n//32,i,protected)for e in ELEMENTS if e!=3))for i,r in enumerate(base['frames'])]
            save(f'aot39-sbr-n{n}-crc{int(protected)}',asc,packets,pcm,2*n,39)
    (OUT/'packets.bin').write_bytes(blob);(OUT/'reference.f32le').write_bytes(gold)
    m=dict(cases=cases,mapping=MAP,positions=[dict(height=h,position=p,index=i,group_channels=n)
        for h,p,n in GROUPS for i in range(n)],scope='Fourteen authored 24-channel programmes; six AUs each. '
        'Independent sparse cosine/window histories, all24 roles retained; two LFEs inside element sequence. '
        'No private media, FFmpeg, network or external decoder. Explicit generation only.')
    (OUT/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
    print('Authored22.2:',len(cases),'programmes;',len(gold)//4,'oracle scalars')
if __name__=='__main__':generate()
