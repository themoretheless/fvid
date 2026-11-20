#!/usr/bin/env python3
"""Own multi-core LD-MPS spatial frames and quantized Matroska clocks."""
import json, math, struct
from functools import lru_cache
from pathlib import Path
from generate_aac_ld_layout_fixtures import LAYOUTS, Oracle, geometry, body as core_body
from generate_he_aac_packet_fixtures import field, frequency, packed, video_fixture
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope
from generate_aac_ld_mps_stp_fixtures import config, body
from generate_aac_ld_mps_ges_fixtures import body as ges_body
from generate_aac_ps_matroska_fixtures import element, number, child
ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT / 'tests/fixtures/playback-errors'
OUT = FIX / 'aac-ld-mps-multicore'
GEOMETRIES = [(480,24000,2),(480,48000,4),(512,24000,2),(512,48000,2),(512,48000,3),(512,48000,4)]

def main():
    OUT.mkdir(exist_ok=True)
    parents = json.loads((FIX/'aac-ld-layout.json').read_text())['cases']
    pb = (FIX/'aac-ld-layout-packets.bin').read_bytes()
    pg = (FIX/'aac-ld-layout-reference.f32le').read_bytes()
    assembly = json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases']
    configs = (FIX/'aac-ld-mps-assemble/configs.bin').read_bytes()
    book = json.loads((FIX/'aac-ld-mps-frame/reshape-codewords.json').read_text())
    packets, gold = bytearray(), bytearray()
    mp4_cases, cases, errors = [], [], []
    source = (FIX/'aac-ld-mps-ld-core/n480-r24000-tree0-v1-plain.mp4').read_bytes()
    track = child(child(source,b'moov'),b'trak')
    stbl = child(child(child(track,b'mdia'),b'minf'),b'stbl')
    entry = child(child(stbl,b'stsd')[8:],b'avc1')
    avcc = child(entry[78:],b'avcC')
    width,height = struct.unpack_from('>HH',entry,24)
    sizes = child(stbl,b'stsz')
    size = struct.unpack_from('>I',sizes,4)[0] or struct.unpack_from('>I',sizes,12)[0]
    offset = struct.unpack_from('>I',child(stbl,b'stco'),8)[0]
    picture = source[offset:offset+size]
    vtrack = element(0xae,number(0xd7,2)+number(0x73c5,2)+number(0x83,1)+element(0x86,b'V_MPEG4/ISO/AVC')+element(0x63a2,avcc)+element(0xe0,number(0xb0,width)+number(0xba,height)))

    @lru_cache(None)
    def core(n,rate,channels,count):
        parent = next(c for c in parents if c['n']==n and c['rate']==rate and c['channels']==channels)
        configuration = parent['configuration']
        band_count, high = geometry(n,rate)
        bins = list(range(4))+list(range(high,high+4))
        cos = [[math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i in range(2*n)] for k in bins]
        banks = [Oracle(n,high,cos) for _ in range(channels)]
        wires, reference = [], bytearray()
        elements, mapping, _ = LAYOUTS[configuration]
        for i in range(count):
            wire, pcm, ch = '', [], 0
            for kind in elements:
                wire += '0000'+('0' if kind==1 else '')
                for _ in range(2 if kind==1 else 1):
                    encoded, params = core_body(n,band_count,high,i,ch,lfe=kind==3)
                    wire += encoded
                    pcm.append(banks[ch].run(*params))
                    ch += 1
            if i < 8:
                row = parent['frames'][i]
                assert packed(wire)==pb[row['offset']:row['offset']+row['bytes']]
            wires.append(wire)
            for t in range(n):
                row = [0.]*channels
                for ch,target in enumerate(mapping): row[target]=pcm[ch][t]
                reference.extend(struct.pack('<'+'f'*channels,*row))
        at = parent['reference_offset']
        assert reference[:8*n*channels*4]==pg[at:at+8*n*channels*4]
        start = len(gold)
        gold.extend(reference)
        return configuration, wires, start

    def build(n,rate,group,tree,variant,mode,bad=None):
        inputs = 1 if tree in (0,1,7) else 2 if tree in (2,3,4) else 6
        count = 4*group
        configuration,wires,ref_at = core(n,rate,inputs,count)
        bins = 32 if rate==24000 else 64
        slots = n*group//bins
        assert n*group%bins==0 and 1<=slots<=32
        shape = {'stp':1,'ges':2}.get(mode,0)
        def select(bands): return next(c for c in assembly if c['tree']==tree and c['variant']==variant and c['bands']==bands)
        def ssc(c): return config(c,rate,slots,configs[c['config_offset']:c['config_offset']+c['config_bytes']],shape)
        initial = ssc(select(23))
        core_asc = field(23,5)+frequency(rate)+field(configuration,4)+field(n==480,1)+'00'+'00'
        asc = packed(core_asc+field(0x7cc,11)+'1'+field(len(initial),8)+raw(initial))
        suffix, refs, previous = ['']*count, [], None
        for i in range(4):
            bands = (23 if i==0 else 9 if i<3 else 4) if mode=='grid' else 23
            c = select(bands)
            current = ssc(c)
            spatial,_ = ges_body(c,i,slots,'alternating',book,True) if shape==2 else body(c,i,slots,'alternating',shape==1)
            if bad=='trailing' and i==1: spatial += b'\xff'
            if bad=='inband-rate' and i==1: current=bytes([((3 if rate==24000 else 6)<<4)|(current[0]&15)])+current[1:]
            header = i==0 or mode=='repeat' or current!=previous
            payload = packed('0'+field(len(current),7))+current+spatial if header else spatial
            suffix[(i+1)*group-1] = envelope(9,1 if header else 0,3,payload,True)
            refs.append(dict(ssc=current.hex(),spatial=spatial.hex(),header=header))
            previous = current
        if mode=='delayed':
            indices = [(i+1)*group-1 for i in (1,2,3)]
            joined = ''.join(suffix[i] for i in indices)
            for i in indices: suffix[i]=''
            suffix[indices[-1]]=joined
        if mode=='fragmented':
            at=2*group-1
            payload=bytes.fromhex(refs[1]['spatial'])
            half=len(payload)//2
            suffix[at]=envelope(9,0,2,payload[:half],True)
            suffix[at+1]=envelope(9,0,1,payload[half:],True)+suffix[at+1]
        rows=[]
        for wire,tail in zip(wires,suffix):
            data=packed(wire+tail)
            rows.append(dict(offset=len(packets),bytes=len(data),core_bits=len(wire)))
            packets.extend(data)
        name=f'n{n}-r{rate}-g{group}-tree{tree}-v{variant}-{mode}'+('-bad-'+bad if bad else '')
        c=dict(name=name,n=n,rate=rate,group=group,core_count=count,slots=slots,bands=bins,tree=tree,variant=variant,mode=mode,channels=inputs,configuration=configuration,output_channels=[6,6,6,8,8,8,8,2][tree],asc=asc.hex(),ssc=initial.hex(),frames=rows,reference=refs,core_reference_offset=ref_at,container_frame_samples=n,container_rate=rate,samples=n*count,pcm_offset=0,video=name+'.mp4')
        (OUT/c['video']).write_bytes(video_fixture([c],packets,inputs,write=False)['data'])
        return c

    def mkv(c,base,scale,duration='floor',bad=False):
        period=c['n']*10**9
        value=period//c['rate'] if duration=='floor' else (period+c['rate']-1)//c['rate']
        default=b'' if duration=='absent' else number(0x23e383,value)
        audio=element(0xe1,element(0xb5,struct.pack('>d',c['rate']))+number(0x9f,c['output_channels']))
        atrack=element(0xae,number(0xd7,1)+number(0x73c5,1)+number(0x83,2)+element(0x86,b'A_AAC')+element(0x63a2,bytes.fromhex(c['asc']))+default+audio)
        clusters=[]
        for i,row in enumerate(c['frames']):
            data=number(0xe7,i*period//(c['rate']*scale))
            if i==0: data+=element(0xa3,b'\x82\0\0\x80'+picture)
            data+=element(0xa3,b'\x81\0\0\x80'+packets[row['offset']:row['offset']+row['bytes']])
            clusters.append(element(0x1f43b675,data))
        header=element(0x1a45dfa3,element(0x4282,b'matroska')+number(0x4287,4)+number(0x4285,2))
        info=element(0x1549a966,number(0x2ad7b1,scale)+element(0x4489,struct.pack('>d',c['samples']*10**9/(c['rate']*scale))))
        name=c['name']+f'-tick{scale}-{duration}'
        file=name+'.mkv'
        (OUT/file).write_bytes(header+element(0x18538067,info+element(0x1654ae6b,atrack+vtrack)+b''.join(clusters)))
        item=dict(name=name,file=file,base=base,n=c['n'],rate=c['rate'],group=c['group'],core_count=c['core_count'],output_channels=c['output_channels'],clock=scale,default_duration=duration,mp4=c['video'])
        if bad: item['error']=c['error']
        return item

    for n,rate,group in GEOMETRIES:
        for tree in range(8):
            for variant in ((0,1) if tree in (2,3,4) else (1,)):
                for mode in ('plain','repeat','delayed','fragmented','stp','ges','grid'):
                    c=build(n,rate,group,tree,variant,mode)
                    index=len(mp4_cases)
                    mp4_cases.append(c)
                    for scale in ((1,1000,1000000) if mode=='plain' else (1000,)):
                        cases.append(mkv(c,index,scale))
                    if mode=='plain' and tree==7:
                        for duration in ('ceil','absent'): cases.append(mkv(c,index,1000,duration))
        for bad,error in [('trailing','trailing bytes after LD-MPS spatial frame'),('inband-rate','in-band output format change')]:
            c=build(n,rate,group,7,1,'plain',bad)
            c['error']=error
            index=len(mp4_cases)
            mp4_cases.append(c)
            errors.append(mkv(c,index,1000,bad=True))
    (OUT/'packets.bin').write_bytes(packets)
    (OUT/'core-reference.f32le').write_bytes(gold)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,errors=errors,mp4_cases=mp4_cases,geometries=GEOMETRIES,provenance='Own AAC-LD/MPS, analytical sparse-cosine core oracle and own AVC. No private media/process/network. All integer multi-core geometries within32 spatial slots at24/48kHz.'),separators=(',',':'))+'\n')
    print(len(cases),'Matroska acceptance;',len(mp4_cases)-len(errors),'MP4 acceptance;',len(errors),'late errors per container')
if __name__=='__main__': main()
