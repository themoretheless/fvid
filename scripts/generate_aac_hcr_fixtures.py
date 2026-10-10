#!/usr/bin/env python3
"""Own HCR encoder and scalar PCM; fixtures are generated offline, never in tests."""
import json,math,re,struct
from generate_aac_main_tools_fixtures import ics,Filterbank,SEQUENCES,sc,f32
from generate_aac_ssr_fixtures import ints,ROOT
from generate_aac_er_sections_fixtures import pair,LIMITS
from generate_aac_rvlc_fixtures import scales
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
MAX=[0,11,9,20,16,13,11,14,12,17,14,49,0,0,0,0,14,17,21,21,25,25,29,29,29,29,33,33,33,37,37,41]
PRIORITY=[99,21,21,20,20,19,19,18,18,17,17,0,99,99,99,99,16,15,14,13,12,11,10,9,8,7,6,5,4,3,2,1]
TABLES={b:(ints(f'SPECTRUM_CODEBOOK{b}_CODES'),ints(f'SPECTRUM_CODEBOOK{b}_LENS')) for b in range(1,12)}
source=(ROOT/'crates/fvid-media/src/owned_aac/aac_band_tables.rs').read_text()
def offsets(seq):
    kind='SHORT' if seq==2 else 'LONG';body=re.search(r'const SWB_OFFSET_24K_'+kind+r':[^=]+ = \[(.*?)\];',source,re.S)[1]
    return [int(v) for v in re.findall(r'\d+',body)][:9]
def cw(book,values):
    if book>=16 or book==11:return pair(*values)
    dim,radix,bias=(4,3,1) if book<=2 else (4,3,0) if book<=4 else (2,9,4) if book<=6 else (2,8,0) if book<=8 else (2,13,0)
    assert len(values)==dim
    index=0
    for v in values:index=index*radix+(abs(v) if bias==0 else v+bias)
    codes,lens=TABLES[book];bits=field(codes[index],lens[index])
    if bias==0:bits+=''.join(field(v<0,1) for v in values if v)
    return bits

def reorder(words):
    if not words:return '',0,dict(segments=0,sets=0,splits=0,shifts=0,tail=0)
    total=sum(len(w['bits']) for w in words);longest=max(len(w['bits']) for w in words);segments=[];start=0;tail=0
    for word in words:
        width=min(MAX[word['book']],longest)
        if start+width<=total:segments.append([start,start+width]);start+=width
        else:
            assert segments
            tail=total-start;segments[-1][1]=total;start=total;break
    assert start==total
    wire=[None]*total;at=[0]*len(words);splits=shifts=0;n=len(segments)
    def write(word,seg,direction):
        nonlocal splits
        begin,end=segments[seg];available=end-begin;take=min(available,len(words[word]['bits'])-at[word])
        for _ in range(take):
            bit=words[word]['bits'][at[word]];at[word]+=1
            if direction==1:wire[begin]=bit;begin+=1
            else:end-=1;wire[end]=bit
        segments[seg]=[begin,end]
        if take and at[word]<len(words[word]['bits']):splits+=1
        return take
    for word in range(n):write(word,word,1);assert at[word]==len(words[word]['bits'])
    sets=(len(words)+n-1)//n
    for group in range(1,sets):
        for trial in range(n):
            for base in range(n):
                word=group*n+base
                if word<len(words) and at[word]<len(words[word]['bits']):
                    take=write(word,(trial+base)%n,-1 if group%2 else 1)
                    if trial and take:shifts+=1
        assert all(at[i]==len(words[i]['bits']) for i in range(group*n,min((group+1)*n,len(words))))
    assert None not in wire and all(a==b for a,b in segments)
    return ''.join(wire),longest,dict(segments=n,sets=sets,splits=splits,shifts=shifts,tail=tail)

def payload(seq,books,groups,q,gain=72,raw=None,longest=None,length=None):
    off=offsets(seq);words=[];first=0
    for g,count in enumerate(groups):
        for band,book in enumerate(books[g]):
            if book==0:continue
            dim=4 if book<=4 else 2
            for window in range(first,first+count):
                for line in range(off[band],off[band+1],4):
                    for half in range(4//dim):
                        values=q[window][line+half*dim:line+(half+1)*dim]
                        words.append(dict(key=(PRIORITY[book],line,window,half),book=book,bits=cw(book,values)))
        first+=count
    words.sort(key=lambda w:w['key']);data,hint,stats=reorder(words)
    return (data if raw is None else raw), (hint if longest is None else longest), stats

def main():
    blob=bytearray();gold=bytearray();regions=bytearray();quantized=bytearray();cases=[];malformed=[];coverage=[]
    for n in (960,1024):
     for section in (False,True):
      for rvlc in (False,True):
       for channels in (1,2):
        banks=[Filterbank(n) for _ in range(channels)];frames=[];start=len(gold)
        for frame,seq in enumerate(SEQUENCES):
            groups=([1,3,4] if frame%2 else [8]) if seq==2 else [1];shape=frame%2
            info=ics(seq,8,False,shape=shape)
            if seq==2 and groups==[1,3,4]:info=info[:-7]+'0110111'
            wire='0000'+('1'+info+field(2 if frame%2 else 0,2) if channels==2 else '');spectra=[];channel_rows=[]
            for ch in range(channels):
                options=list(range(1,12))+(list(range(16,32)) if section else [])
                books=[[options[(frame*8+g*3+b+ch)%len(options)] if (frame+g+b)%11 else 0 for b in range(8)] for g in range(len(groups))]
                if frame==0:books=[[0]*8 for _ in groups]
                off=offsets(seq);q=[[0]*off[-1] for _ in range(8 if seq==2 else 1)];first=0
                for g,count in enumerate(groups):
                    for band,book in enumerate(books[g]):
                        if not book:continue
                        amp=1 if book<=2 else 2 if book<=4 else 4 if book<=6 else 7 if book<=8 else 12 if book<=10 else 8191 if book==11 else LIMITS[book-16]
                        for w in range(first,first+count):
                            for k in range(off[band],off[band+1]):
                                q[w][k]=(amp if (w+k+frame+ch)%2 else -amp) if (frame+w+k//4+ch)%4==0 else 0
                    first+=count
                data,hint,stats=payload(seq,books,groups,q);coverage.append(stats)
                prefix=(-len(data))%8;raw_region=packed('0'*prefix+data)
                reference=[];first=0
                for g,count in enumerate(groups):
                    for band in range(8):
                        for w in range(first,first+count):reference.extend(q[w][off[band]:off[band+1]])
                    first+=count
                channel_rows.append(dict(offset=len(regions),bytes=len(raw_region),prefix=prefix,bits=len(data),longest=hint,groups=groups,books=books,offsets=off+[n//(8 if seq==2 else 1)],sequence=seq,reference_offset=len(quantized),reference_bytes=len(reference)*2))
                regions.extend(raw_region);quantized.extend(struct.pack('<'+str(len(reference))+'h',*reference))
                flat=[b for group in books for b in group]
                sections=''.join(field(b,5 if section else 4)+('' if section and (b==11 or b>=16) else field(1,3 if seq==2 else 5)) for b in flat)
                header,rvdata,_=scales(seq,flat,[72 if b else 0 for b in flat],gain=72) if rvlc else (sc(0)*sum(b!=0 for b in flat),'',None)
                # Deferred ER TNS body is after class2 RVLC, before HCR data.
                tns=seq!=2 and ch==0
                body=field(72,8)+('' if channels==2 else info)+sections+header+'0'+field(tns,1)+'0'+field(len(data),14)+field(hint,6)+rvdata
                if tns:body+=field(1,2)+'0'+field(47,6)+field(1,5)+'00'+'001'
                wire+=body+data
                spectrum=[[f32(math.copysign(abs(v)**(4/3)*2**(-7),v)) if v else 0. for v in row] for row in q]
                spectra.append(spectrum)
            if channels==2 and frame%2:
                for w in range(len(spectra[0])):
                    for k in range(len(spectra[0][w])):
                        a,b=spectra[0][w][k],spectra[1][w][k];spectra[0][w][k]=f32(a+b);spectra[1][w][k]=f32(a-b)
            if seq!=2:
                previous=0.
                for k in range(len(spectra[0][0])):
                    v=f32(spectra[0][0][k]-math.sin(math.pi/7)*previous);spectra[0][0][k]=v;previous=v
            packet=packed(wire);frames.append(dict(offset=len(blob),bytes=len(packet),padding=(-len(wire)%8),hcr_channels=channel_rows));blob.extend(packet)
            lanes=[banks[ch].run(seq,spectra[ch],shape) for ch in range(channels)]
            for i in range(n):gold.extend(struct.pack('<'+str(channels)+'f',*(lane[i] for lane in lanes)))
        flags=1+4*section+2*rvlc;asc=packed(field(17,5)+frequency(24000)+field(channels,4)+field(n==960,1)+'01'+field(flags,3)+'000').hex()
        c=dict(name=f'{n}-{int(section)}-{int(rvlc)}-{channels}',n=n,channels=channels,asc=asc,frames=frames,reference_offset=start,reference_bytes=12*n*channels*4,slots=n//64,bands=32,container_rate=24000,container_frame_samples=n,samples=12*n,pcm_offset=0)
        c['video']=video_fixture([c],blob,channels=channels,filename=f'aac-hcr-{c["name"]}-synthetic.mp4');cases.append(c)
        if channels!=1:continue
        variants=[('zero-longest','0',0,1,'AAC HCR nonempty region has zero longest codeword'),('no-segment','0',49,1,'AAC HCR region cannot hold a priority segment'),('unused-bits','00',1,2,'AAC HCR unused spectral bits'),('truncated','0',1,16383,'truncated AAC HCR spectral region'),('short-longest','1'*10,10,10,'AAC HCR codeword exceeds declared longest'),('nonpriority','0'+'1'*10,11,11,'AAC HCR incomplete nonpriority codeword')]
        for kind,data,hint,length,error in variants:
            # One zero quad in band0; class2 region deliberately malformed.
            bands=2 if kind in ('short-longest','nonpriority') else 1
            header,rvdata,_=scales(0,[1]*bands,[72]*bands,gain=72) if rvlc else (sc(0)*bands,'',None)
            sections=(field(1,5 if section else 4)+field(1,5))*bands
            raw=packed('0000'+field(72,8)+ics(0,bands,False)+sections+header+'000'+field(length,14)+field(hint,6)+rvdata+data)
            row=dict(offset=len(blob),bytes=len(raw),padding=0);blob.extend(raw)
            video=video_fixture([dict(c,frames=[row],samples=n)],blob,channels=1,filename=f'aac-hcr-bad-{c["name"]}-{kind}-synthetic.mp4')
            malformed.append(dict(kind=kind,asc=asc,frame=row,video=video,error=error))
        if section:
            data,hint,_=payload(0,[[17]],[1],[[32,0,0,0]])
            header,rvdata,_=scales(0,[17],[72],gain=72) if rvlc else (sc(0),'',None)
            raw=packed('0000'+field(72,8)+ics(0,1,False)+field(17,5)+header+'000'+field(len(data),14)+field(hint,6)+rvdata+data)
            row=dict(offset=len(blob),bytes=len(raw),padding=0);blob.extend(raw)
            video=video_fixture([dict(c,frames=[row],samples=n)],blob,channels=1,filename=f'aac-hcr-bad-{c["name"]}-virtual-lav-synthetic.mp4')
            malformed.append(dict(kind='virtual-lav',asc=asc,frame=row,video=video,error='AAC virtual codebook magnitude exceeds section limit'))
    assert max(v['sets'] for v in coverage)>=3 and sum(v['splits'] for v in coverage)>0 and sum(v['shifts'] for v in coverage)>0
    (DEST/'aac-hcr-regions.bin').write_bytes(regions);(DEST/'aac-hcr-quantized.i16le').write_bytes(quantized)
    (DEST/'aac-hcr-packets.bin').write_bytes(blob);(DEST/'aac-hcr-reference.f32le').write_bytes(gold)
    (DEST/'aac-hcr.json').write_text(json.dumps(dict(cases=cases,malformed=malformed,coverage=coverage,provenance='Own codeword sorting, segmented PCW and alternating-direction shifted non-PCW encoder; all physical and virtual books; independent scalar quantization/MS/TNS/direct IMDCT/window PCM. No private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
