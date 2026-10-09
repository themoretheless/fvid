#!/usr/bin/env python3
"""Authored opaque non-VCL NALs added to an existing synthetic HEVC fixture."""
import struct,json,hashlib
from generate_he_aac_packet_fixtures import DEST,boxes
from hevc_fixture_mp4 import box

def child(data,tag):return next(p for t,p in boxes(data) if t==tag)
def main():
    seed=(DEST/'hevc-multislice-main.mp4').read_bytes()
    root=dict(boxes(seed));track=child(root[b'moov'],b'trak')
    stbl=child(child(child(track,b'mdia'),b'minf'),b'stbl')
    sizes=child(stbl,b'stsz');constant,count=struct.unpack_from('>II',sizes,4);assert constant==0
    lengths=struct.unpack('>'+str(count)+'I',sizes[12:])
    offset=struct.unpack_from('>I',child(stbl,b'stco'),8)[0]
    assert struct.unpack_from('>III',child(stbl,b'stsc'),8)==(1,count,1)
    packets=[];pos=offset
    for n in lengths:
        p=seed[pos:pos+n];pos+=n;units=[];j=0
        while j<len(p):
            k=int.from_bytes(p[j:j+4],'big');j+=4;assert k and j+k<=len(p)
            units.append(p[j:j+k]);j+=k
        assert len(units)==2;packets.append(units)
    cases=[];invalid=[]
    for size in [2,4]:
        for position in ['prefix','between','suffix','bad-temporal','bad-layer','bad-forbidden']:
            if size!=4 and position.startswith('bad-'):continue
            opaque=[bytes([kind<<1,1,kind,128]) for kind in range(41,64)]
            if position=='bad-temporal':opaque[0]=bytes([82,0,41,128])
            elif position=='bad-layer':opaque[0]=bytes([82,9,41,128])
            elif position=='bad-forbidden':opaque[0]=bytes([210,1,41,128])
            output=[]
            for units in packets:
                location={'prefix':0,'between':1,'suffix':2}.get(position,0)
                selected=units[:location]+opaque+units[location:]
                output.append(b''.join(len(n).to_bytes(size,'big')+n for n in selected))
            def rewrite(tag,payload):
                if tag in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                    payload=b''.join(rewrite(t,p) for t,p in boxes(payload))
                elif tag==b'stsz':payload=payload[:12]+b''.join(struct.pack('>I',len(p)) for p in output)
                elif tag==b'stco':payload=payload[:8]+struct.pack('>I',len(root[b'ftyp'])+16)
                elif tag==b'stsd':
                    kind,entry=next(boxes(payload[8:]));assert kind in [b'hvc1',b'hev1']
                    config=bytearray(child(entry[78:],b'hvcC'));config[21]=(config[21]&252)|(size-1)
                    payload=payload[:8]+box(kind,entry[:78]+b''.join(box(t,bytes(config) if t==b'hvcC' else p) for t,p in boxes(entry[78:])))
                return box(tag,payload)
            movie=box(b'ftyp',root[b'ftyp'])+box(b'mdat',b''.join(output))+rewrite(b'moov',root[b'moov'])
            name=f'hevc-opaque-non-vcl-length{size}-{position}-synthetic.mp4';(DEST/name).write_bytes(movie)
            row=dict(file=name,length_size=size,position=position,frames=count,types=list(range(41,64)),sha256=hashlib.sha256(movie).hexdigest())
            if position.startswith('bad-'):
                row['error']='HEVC multilayer decoding is not implemented' if position=='bad-layer' else 'invalid HEVC forbidden/temporal header bits'
                invalid.append(row)
            else:cases.append(row)
    (DEST/'hevc-opaque-non-vcl.json').write_text(json.dumps(dict(seed='hevc-multislice-main.mp4',seed_sha256=hashlib.sha256(seed).hexdigest(),cases=cases,invalid=invalid),indent=2)+'\n')
    print(len(cases),'videos;',count,'frames; all 23 reserved/unspecified non-VCL types')
if __name__=='__main__':main()
