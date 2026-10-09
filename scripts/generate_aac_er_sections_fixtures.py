#!/usr/bin/env python3
"""Own ER section-resilience / virtual escape books; offline scalar oracle."""
import json, math, struct
from generate_aac_main_tools_fixtures import ics, Filterbank, SEQUENCES, sc, tuple_bits
from generate_aac_ssr_fixtures import ints
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
LIMITS=[15,31,47,63,95,127,159,191,223,255,319,383,511,767,1023,2047]
CODES=ints('SPECTRUM_CODEBOOK11_CODES'); LENS=ints('SPECTRUM_CODEBOOK11_LENS')
def pair(a,b):
    index=min(abs(a),16)*17+min(abs(b),16)
    bits=field(CODES[index],LENS[index])+''.join(field(v<0,1) for v in (a,b) if v)
    for v in (a,b):
        v=abs(v)
        if v>=16:
            width=v.bit_length()-1
            bits+='1'*(width-4)+'0'+field(v-(1<<width),width)
    return bits

def main():
    blob=bytearray();gold=bytearray();cases=[];malformed=[]
    # Every virtual book, including its exact maximum, both signs and escape
    # boundaries; an explicit-length book1 and implicit-length book11 share ICS.
    for n in (960,1024):
        for book in range(16,32):
            bank=Filterbank(n);rows=[];start=len(gold)
            for frame,seq in enumerate(SEQUENCES):
                shape=frame%2;limit=LIMITS[book-16]
                q=[[1,-1,0,1, limit,-limit,0, min(16,limit),16,-31,1,0] for _ in range(8 if seq==2 else 1)]
                wire='0000'+field(100,8)+ics(seq,3,False,shape=shape)
                wire+=field(1,5)+field(1,3 if seq==2 else 5)+field(book,5)+field(11,5)
                wire+=sc(0)*3+'000'
                for band in range(3):
                    for row in q:
                        values=row[4*band:4*band+4]
                        wire+=tuple_bits(values) if band==0 else pair(*values[:2])+pair(*values[2:])
                raw=packed(wire);rows.append(dict(offset=len(blob),bytes=len(raw),padding=(-len(wire)%8)));blob.extend(raw)
                spectra=[[math.copysign(abs(v)**(4/3),v) if v else 0. for v in row] for row in q]
                gold.extend(struct.pack('<'+str(n)+'f',*bank.run(seq,spectra,shape)))
            asc=packed(field(17,5)+frequency(24000)+'0001'+field(n==960,1)+'01100000').hex()
            c=dict(name=f'{n}-{book}',n=n,book=book,channels=1,asc=asc,frames=rows,reference_offset=start,reference_bytes=12*n*4,slots=n//64,bands=32,container_rate=24000,container_frame_samples=n,samples=12*n,pcm_offset=0)
            c['video']=video_fixture([c],blob,channels=1,filename=f'aac-er-sections-{n}-{book}-synthetic.mp4');cases.append(c)
            # Exact virtual LAV violation, fully valid Huffman/escape payload.
            wire='0000'+field(100,8)+ics(0,1,False)+field(book,5)+sc(0)+'000'+pair(limit+1,0)+pair(0,0)
            raw=packed(wire);row=dict(offset=len(blob),bytes=len(raw),padding=(-len(wire)%8));blob.extend(raw)
            bad=dict(c,frames=[row],samples=n)
            video=video_fixture([bad],blob,channels=1,filename=f'aac-er-sections-overflow-{n}-{book}-synthetic.mp4')
            malformed.append(dict(video=video,frame=row,asc=asc,error='AAC virtual codebook magnitude exceeds section limit'))
    (DEST/'aac-er-sections-packets.bin').write_bytes(blob)
    (DEST/'aac-er-sections-reference.f32le').write_bytes(gold)
    (DEST/'aac-er-sections.json').write_text(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own AOT17 section-resilience bits, virtual books16..31 exact LAV and signed escape magnitudes, mixed explicit and implicit section lengths, 960/1024 sine/KBD transitions; direct scalar IMDCT/window oracle. No private media, foreign encoder, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
