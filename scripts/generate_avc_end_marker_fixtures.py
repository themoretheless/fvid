#!/usr/bin/env python3
"""Append authored AVC end markers to an existing synthetic multi-slice video."""
from pathlib import Path
import struct,json,hashlib
from generate_he_aac_packet_fixtures import boxes,DEST
from avc_fixture_mp4 import mux

def child(data,kind):return next(body for tag,body in boxes(data) if tag==kind)
def words(data):return struct.unpack('>'+str(len(data)//4)+'I',data)
def main():
    seed=(DEST/'avc-multislice-ipb.mp4').read_bytes()
    trak=child(child(seed,b'moov'),b'trak');mdia=child(trak,b'mdia')
    stbl=child(child(mdia,b'minf'),b'stbl')
    entry=child(child(stbl,b'stsd')[8:],b'avc1')
    config=child(entry[78:],b'avcC');assert config[4]&3==3
    width,height=struct.unpack_from('>HH',entry,24)
    rate=struct.unpack_from('>I',child(mdia,b'mdhd'),12)[0]
    size=child(stbl,b'stsz');constant,count=struct.unpack_from('>II',size,4)
    sizes=[constant]*count if constant else words(size[12:]);assert len(sizes)==count
    offsets=words(child(stbl,b'stco')[8:]);mapping=words(child(stbl,b'stsc')[8:]);assert mapping== (1,count,1) and len(offsets)==1
    dur=words(child(stbl,b'stts')[8:]);assert dur==(count,1)
    ctts=child(stbl,b'ctts');assert ctts[0]==1
    deltas=[]
    for pos in range(8,len(ctts),8):
        n,delta=struct.unpack_from('>Ii',ctts,pos);deltas += [delta]*n
    keys=set(words(child(stbl,b'stss')[8:]));frames=[];position=offsets[0]
    for i,n in enumerate(sizes):
        frames.append((i+deltas[i],i+1 in keys,seed[position:position+n]));position+=n
    rows=[]
    for length_size in [2,4]:
        repacked=[]
        for pts,key,packet in frames:
            pos=0;out=bytearray()
            while pos<len(packet):
                size=int.from_bytes(packet[pos:pos+4],'big');pos+=4
                assert size and pos+size<=len(packet) and size<1<<(8*length_size)
                out.extend(size.to_bytes(length_size,'big'));out.extend(packet[pos:pos+size]);pos+=size
            repacked.append((pts,key,bytes(out)))
        modified_config=bytearray(config);modified_config[4]=(config[4]&252)|(length_size-1)
        for name,markers in [('sequence',[10]),('stream',[11]),('both',[10,11])]:
            # End markers follow the final VCL AU; neither marker introduces a picture.
            tail=b''.join((2).to_bytes(length_size,'big')+bytes([kind,128]) for kind in markers)
            marked=repacked[:-1]+[(repacked[-1][0],repacked[-1][1],repacked[-1][2]+tail)]
            out=mux(bytes(modified_config),marked,width,height,rate)
            filename=f'avc-end-{name}-length{length_size}-synthetic.mp4';(DEST/filename).write_bytes(out)
            rows.append(dict(file=filename,markers=markers,length_size=length_size,frames=count,sha256=hashlib.sha256(out).hexdigest()))
    (DEST/'avc-end-markers.json').write_text(json.dumps(dict(seed='avc-multislice-ipb.mp4',seed_sha256=hashlib.sha256(seed).hexdigest(),cases=rows),indent=2)+'\n')
    print(len(rows),'original marker videos;',count,'frames each')
if __name__=='__main__':main()
