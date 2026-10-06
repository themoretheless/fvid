#!/usr/bin/env python3
"""Owned PCM/P/B streams forcing cross-mode temporal direct from B motion."""
import argparse, hashlib, json, re, subprocess, tempfile
from pathlib import Path
from avc_fixture_mp4 import mux, annexb

class Writer:
    def __init__(self): self.bits=[]
    def u(self, value, width=1):
        assert 0 <= value < 1 << width
        self.bits.extend((value >> shift) & 1 for shift in range(width-1,-1,-1))
    def ue(self, value):
        code=value+1; width=code.bit_length()
        self.bits.extend([0]*(width-1)); self.u(code,width)
    def se(self, value): self.ue(2*value-1 if value>0 else -2*value)
    def align(self):
        while len(self.bits)%8: self.u(0)
    def nal(self, header, trailing=True):
        if trailing: self.u(1)
        self.align()
        raw=bytes(sum(self.bits[i+j] << (7-j) for j in range(8)) for i in range(0,len(self.bits),8))
        escaped=bytearray([header]); zeros=0
        for value in raw:
            if zeros==2 and value<=3: escaped.append(3); zeros=0
            escaped.append(value); zeros=zeros+1 if value==0 else 0
        return bytes(escaped)

class CabacWriter:
    """Integer interval encoder for the short owned B motion/direct syntax (H.264 9.3)."""
    def __init__(self, init_idc, qp=26):
        root=Path(__file__).resolve().parents[1]/'src/codec'
        source=(root/'cabac_tables.rs').read_text()
        def table(name):
            body=source.split('const '+name+':',1)[1].split('= ',1)[1].split(';',1)[0]
            return list(map(int,re.findall(r'\d+',body)))
        self.lps=table('RANGE_LPS'); self.tm=table('TRANS_MPS'); self.tl=table('TRANS_LPS')
        entries=re.findall(r'Some\(\((-?\d+), (-?\d+)\)\)|None',(root/'avc_cabac_init.rs').read_text().split('= [',1)[1])
        assert len(entries)==1840
        self.contexts=[]
        for m,n in entries[460*(init_idc+1):460*(init_idc+2)]:
            if not m: self.contexts.append(None); continue
            pre=max(1,min(126,(int(m)*max(0,min(51,qp)) >> 4)+int(n)))
            self.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
        assert len(self.contexts)==460
        self.low=0; self.range=510; self.width=9
    def renormalize(self):
        while self.range<256:
            self.range*=2; self.low*=2; self.width+=1
    def decision(self,index,value):
        state,mps=self.contexts[index]
        lps=self.lps[state*4+((self.range>>6)&3)]; self.range-=lps
        if value!=mps:
            self.low+=self.range; self.range=lps
            self.contexts[index]=[self.tl[state],mps ^ int(state==0)]
        else: self.contexts[index]=[self.tm[state],mps]
        self.renormalize()
    def bypass(self,value):
        self.low=self.low*2+(self.range if value else 0); self.width+=1
    def mvd(self,component,value,neighbour=0):
        offset=40 if component==0 else 47
        increment=0 if neighbour<3 else 2 if neighbour>32 else 1
        magnitude=abs(value)
        assert magnitude<9 # owned source uses 8/4; no escape needed
        for symbol in range(magnitude+1):
            self.decision(offset+(increment if symbol==0 else min(symbol+2,6)),int(symbol<magnitude))
        if magnitude: self.bypass(int(value<0))
    def finish(self):
        self.low+=self.range-2; self.range=2
        # Select the odd terminal point: valid offset plus the RBSP stop bit.
        code=self.low | 1
        return [(code >> shift)&1 for shift in range(self.width-1,-1,-1)]

def direct_cabac(field, init_idc, top_skip, qp=26):
    b=CabacWriter(init_idc,qp)
    for address in range(2):
        b.decision(24+int(address==1 and not field and not top_skip),int(address==0 and top_skip))
        if address==0 and top_skip: continue
        if address==0 or top_skip: b.decision(70,int(field))
        b.decision(27,0) # B_Direct_16x16; both neighbours direct/unavailable
        for ctx in ([75,76,75,76] if address==1 and not field else [73,74,75,76]):
            b.decision(ctx,0)
        b.decision(77,0) # chroma coded_block_pattern=0
    return b.finish()

def explicit_b_cabac(field, init_idc, qp=26, active_l0=1):
    b=CabacWriter(init_idc,qp)
    for address in range(2):
        frame_bottom=address==1 and not field
        b.decision(24+int(frame_bottom),0)
        if address==0: b.decision(70,int(field))
        # B_L0_16x16 (100): bottom frame's top neighbour is non-direct.
        b.decision(27+int(frame_bottom),1); b.decision(30,0); b.decision(32,0)
        if field:
            b.decision(54,1); b.decision(58,0) # expanded reference 1
        elif active_l0>1: b.decision(54,0)
        vector=(0,0) if frame_bottom else (8,4)
        for component, value in enumerate(vector):
            b.mvd(component,value,(8,4)[component] if frame_bottom else 0)
        for ctx in ([75,76,75,76] if frame_bottom else [73,74,75,76]): b.decision(ctx,0)
        b.decision(77,0)
    return b.finish()

def pcm_samples(depth,address,first_mb):
    b=Writer()
    for component in range(3):
        size=16 if component==0 else 8
        for y in range(size):
            for x in range(size):
                row=address*size+y
                value=24+((x+first_mb*size)*3+row*5+component*37)%176
                if depth>8: value=(value << (depth-8))+(x+row+component)%(1 << (depth-8))
                b.u(value,depth)
    return b.bits

def pcm_cabac(depth,field,first_mb,qp=26):
    b=CabacWriter(-1,qp); bits=[] # I slices always use context bank zero
    for address in range(2):
        if address==0: b.decision(70,int(field))
        b.decision(3+int(address==1 and not field),1) # I_PCM escape
        bits.extend(b.finish())
        while len(bits)%8: bits.append(0)
        bits.extend(pcm_samples(depth,address,first_mb))
        # PCM restarts arithmetic while retaining adapted context probabilities.
        b.low=0; b.range=510; b.width=9
    bits.extend(b.finish()) # end_of_slice_flag after the bottom PCM block
    return bits

def explicit_p_cabac(field,init_idc,qp=26,active_l0=1):
    b=CabacWriter(init_idc,qp)
    for address in range(2):
        frame_bottom=address==1 and not field
        b.decision(11+int(frame_bottom),0)
        if address==0: b.decision(70,int(field))
        b.decision(14,0); b.decision(15,0); b.decision(16,0) # P_L0_16x16
        if field: b.decision(54,1); b.decision(58,0)
        elif active_l0>1: b.decision(54,0)
        vector=(0,0) if frame_bottom else (8,4)
        for component,value in enumerate(vector): b.mvd(component,value,(8,4)[component] if frame_bottom else 0)
        for ctx in ([75,76,75,76] if frame_bottom else [73,74,75,76]): b.decision(ctx,0)
        b.decision(77,0)
    return b.finish()

def config(depth, explicit, cabac_target=False, width_mbs=1, gaps_allowed=False, poc_type=0):
    profile=77 if depth==8 else 110 if depth==10 else 244
    b=Writer(); b.u(profile,8); b.u(0,8); b.u(10,8); b.ue(0)
    if depth>8:
        b.ue(1); b.ue(depth-8); b.ue(depth-8); b.u(0); b.u(0)
    b.ue(0); b.ue(poc_type) # log2_max_frame_num_minus4, pic_order_cnt_type
    if poc_type==0: b.ue(0)
    elif poc_type==1:
        b.u(1); b.se(0); b.se(0); b.ue(1); b.se(2) # zero deltas, cycle offset 2
    else: assert poc_type==2
    b.ue(3); b.u(int(gaps_allowed)); b.ue(width_mbs-1); b.ue(0)
    b.u(0); b.u(1); b.u(1); b.u(0); b.u(0)
    sps=b.nal(0x67)
    pps=[]
    for entropy in range(1+int(cabac_target)):
        b=Writer(); b.ue(entropy); b.ue(0); b.u(entropy); b.u(0); b.ue(0); b.ue(0); b.ue(0)
        b.u(0); b.u(1 if explicit else 2,2); b.se(0); b.se(0); b.se(0); b.u(1); b.u(0); b.u(0)
        pps.append(b.nal(0x68))
    return bytes([1,profile,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([len(pps)])+b''.join(len(p).to_bytes(2,'big')+p for p in pps)

def picture(depth, kind, frame_num, poc, field, reference, explicit, cabac=False, init_idc=0, top_skip=False, first_mb=0, idr=None, deblock=1, qp=26, l0_to_idr=False, filter_offsets=(0,0), poc_type=0, active_l0=1, selected_ref=None, l1_to_idr=False):
    idr=(kind=='I') if idr is None else idr
    b=Writer(); b.ue(first_mb); b.ue(2 if kind=='I' else 0 if kind=='P' else 1); b.ue(int(cabac))
    b.u(frame_num,4); b.u(0)
    if idr: b.ue(0)
    if poc_type==0: b.u(poc,4)
    if kind.startswith('B'): b.u(0) # temporal direct
    if kind!='I':
        b.u(int(active_l0!=1))
        if active_l0!=1:
            b.ue(active_l0-1)
            if kind.startswith('B'): b.ue(active_l0-1)
        b.u(int(l0_to_idr))
        if l0_to_idr:
            assert frame_num>0
            b.ue(0); b.ue(frame_num-1); b.ue(3) # subtract to frame_num 0
        if kind.startswith('B'):
            b.u(int(l1_to_idr))
            if l1_to_idr:
                b.ue(0); b.ue(frame_num-1); b.ue(3)
    if explicit and kind.startswith('B'):
        b.ue(1); b.ue(1) # luma/chroma weight denominators
        for weight, offsets in [(3,(1,2,-1)), (1,(-3,-2,3))]:
            b.u(1); b.se(weight); b.se(offsets[0]); b.u(1)
            for offset in offsets[1:]: b.se(weight); b.se(offset)
    if idr: b.u(0); b.u(0)
    elif reference: b.u(0)
    if cabac and kind!='I': b.ue(init_idc) # cabac_init_idc
    b.se(qp-26); b.ue(deblock) # PPS initial QP 26
    if deblock!=1:
        for offset in filter_offsets:
            assert offset%2==0 and -12<=offset<=12
            b.se(offset//2)
    if cabac:
        while len(b.bits)%8: b.u(1)
        payload=pcm_cabac(depth,field,first_mb,qp) if kind=='I' else explicit_p_cabac(field,init_idc,qp,active_l0) if kind=='P' else direct_cabac(field,init_idc,top_skip,qp) if kind=='Bdirect' else explicit_b_cabac(field,init_idc,qp,active_l0)
        b.bits.extend(payload)
        return b.nal(0x65 if idr else 0x41 if reference else 0x01, trailing=False)
    for address in range(2):
        if kind!='I': b.ue(0) # no skipped macroblocks
        if address==0: b.u(int(field))
        if kind=='I':
            b.ue(25); b.align() # owned I_PCM source
            b.bits.extend(pcm_samples(depth,address,first_mb))
        else:
            b.ue(0 if kind in ['P','Bdirect'] else 1)
            if kind!='Bdirect':
                if active_l0>1: b.ue(selected_ref if selected_ref is not None else 1 if field else 0)
                elif field: b.u(0) # expanded ref_idx_l0=1 (opposite parity)
                vector=(8,4) if address==0 or field else (0,0)
                b.se(vector[0]); b.se(vector[1])
            b.ue(0) # inter coded_block_pattern=0
    header=0x65 if idr else (0x61 if kind=='P' else 0x41 if reference else 0x01)
    return b.nal(header)

def main():
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--jm-decoder',type=Path,required=True); a=p.parse_args()
    output=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'; records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-owned-mbaff-direct-') as temporary:
        directory=Path(temporary)
        for depth in [8,10,12,14]:
            for source_field, explicit, cabac, init_idc, top_skip, source_cabac in [(f,e,c,i,k,s) for f in [False,True] for e in [False,True] for c,i,k in [(False,0,False),(True,0,False),(True,1,False),(True,2,False),(True,0,True)] for s in ([False,True] if c else [False])]:
                target_field=not source_field; configuration=config(depth, explicit, cabac)
                nals=[picture(depth,'I',0,0,False,True,explicit), picture(depth,'P',1,8,False,True,explicit),
                      picture(depth,'Bexplicit',2,4,source_field,True,explicit,source_cabac,init_idc),
                      picture(depth,'Bdirect',3,2,target_field,False,explicit,cabac,init_idc,top_skip),
                      picture(depth,'Bexplicit',3,6,target_field,False,explicit)]
                frames=[(pts,index==0,len(nal).to_bytes(4,'big')+nal) for index,(pts,nal) in enumerate(zip([0,4,2,1,3],nals))]
                name='avc-mbaff-colocated-'+('field-to-frame' if source_field else 'frame-to-field')+('-source-cabac' if source_cabac else '')+('-top-skip' if top_skip else '')+('-init'+str(init_idc) if init_idc else '')+('-explicit' if explicit else '')+('-high'+str(depth) if depth>8 else '')+('-cabac' if cabac else '-cavlc')
                coded=directory/(name+'.264'); oracle=directory/(name+'.yuv'); cfg=directory/'decoder.cfg'; cfg.write_text('')
                coded.write_bytes(annexb(configuration,frames))
                subprocess.run([str(a.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
                pixels=oracle.read_bytes(); assert len(pixels)==5*16*32*3//2*(2 if depth>8 else 1)
                data=mux(configuration,frames,16,32,25)
                (output/(name+'.mp4')).write_bytes(data); (output/(name+'.yuv')).write_bytes(pixels)
                records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (output/'avc-mbaff-direct-generated.json').write_text(json.dumps(dict(generator='owned PCM/header/CAVLC and B-motion/direct CABAC interval writer; no external encoder',jm_decoder_sha256=hashlib.sha256(a.jm_decoder.read_bytes()).hexdigest(),fixtures=records),indent=2)+'\n')
if __name__=='__main__': main()
