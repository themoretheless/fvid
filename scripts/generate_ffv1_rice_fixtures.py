#!/usr/bin/env python3
"""Authored RFC 9043 Rice samples, integer coding traces and owned Matroska; offline."""
import hashlib
import json
from generate_ffv1_rgb_fixtures import Range, element, uint
from pathlib import Path
D = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
J = [0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3,4,4,5,5,6,6,7,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24]
def signed(v, bits):
    v &= (1 << bits) - 1
    return v - (1 << bits) if v & (1 << (bits - 1)) else v
class Rice:
    def __init__(self): self.bits = ''; self.models = [[dict(count=1, total=4, drift=0, bias=0) for _ in range(2)] for _ in range(2)]; self.trace=[]
    def put(self, value, count):
        assert 0 <= value < (1 << count) if count else value == 0
        if count: self.bits += f'{value:0{count}b}'
    def scalar(self, target, bits, group, context):
        m = self.models[group][abs(context)]
        k = 0
        while (m['count'] << k) < m['total']: k += 1
        v = signed(target - m['bias'], bits)
        raw = -1 - v if 2 * m['drift'] < -m['count'] else v
        u = 2 * raw if raw >= 0 else -2 * raw - 1
        start = len(self.bits)
        escape = u >> k >= 12
        if escape: self.put(0, 12); self.put(u - 11, bits)
        else: self.bits += '0' * (u >> k) + '1'; self.put(u & ((1 << k) - 1), k)
        self.trace.append(dict(kind='scalar',k=k,value=target,corrected=v,escape=escape,context=context,group=group,offset=start,length=len(self.bits)-start,state=dict(m)))
        m['total'] += abs(v); m['drift'] += v
        if m['count'] == 128:
            for key in ('count','total','drift'): m[key] >>= 1
        m['count'] += 1
        if m['drift'] <= -m['count']:
            m['bias'] = max(-128,m['bias']-1); m['drift'] = max(m['drift']+m['count'],-m['count']+1)
        elif m['drift'] > 0:
            m['bias'] = min(127,m['bias']+1); m['drift'] = min(m['drift']-m['count'],0)
    def line(self, differences, contexts, bits, group, index):
        x=0; w=len(differences)
        while x<w:
            if contexts[x] != 0:
                self.scalar(differences[x],bits,group,contexts[x]); x+=1; continue
            end=x
            while end<w and differences[end]==0: end+=1
            while x<end:
                quantum=1<<J[index]
                if end-x>=quantum or end==w:
                    self.put(1,1); self.trace.append(dict(kind='run',x=x,index=index,quantum=quantum))
                    if x+quantum<=w:index+=1
                    x=min(x+quantum,w)
                else:break
            if x==w:break
            tail=end-x
            self.put(0,1);self.put(tail,J[index]);index=max(0,index-1);x=end
            value=differences[x]
            assert value!=0
            self.scalar(value-1 if value>0 else value,bits,group,contexts[x]);x+=1
        return index
    def finish(self):
        self.bits += '0' * (-len(self.bits)%8)
        return bytes(int(self.bits[i:i+8],2) for i in range(0,len(self.bits),8))
def header(depth, rgb, chroma, shifts, varied, key, version=1):
    r=Range(); r.bit([128],0,key)
    if key:
        h=[128]*32
        for v in [version,0,int(rgb)]+([depth] if version else []): r.integer(h,v)
        r.bit(h,0,chroma); r.integer(h,shifts[0]);r.integer(h,shifts[1]);r.bit(h,0,False)
        for i in range(5):
            model=[128]*32
            for v in ([0,126] if varied and i==0 else [127]):r.integer(model,v)
    r.bit([129],0,False)
    return r.finish()
def coded_line(p,w,y,bits,varied,signed_samples=None):
    if signed_samples is None:signed_samples=bits==16
    def read(x,y):
        if y<0 or x < -1:return 0
        if x == -1:return 0 if y==0 else read(0,y-1)
        value=p[y*w+min(x,w-1)]
        return signed(value,16) if signed_samples else value
    diffs=[];contexts=[]
    for x in range(w):
        l,t,tl=read(x-1,y),read(x,y-1),read(x-1,y-1)
        prediction=sorted([l,t,l+t-tl])[1]
        q=(l-tl)&255
        context=(1 if 0<q<128 else -1 if q>=128 else 0) if varied else 0
        diff=signed(p[y*w+x]-prediction,bits)
        diffs.append(-diff if context<0 else diff);contexts.append(context)
    return diffs,contexts
def movie(packets,w,h,rgb):
    colour=element(0x55b0,uint(0x55b1,0 if rgb else 6)+uint(0x55b9,2))
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_FFV1')+uint(0x23e383,20000000)+element(0xe0,uint(0xb0,w)+uint(0xba,h)+colour)
    cluster=uint(0xe7,0)+b''.join(element(0xa3,b'\x81'+(i*20).to_bytes(2,'big')+(b'\x80' if i==0 else b'\0')+p) for i,p in enumerate(packets))
    return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000))+element(0x1654ae6b,element(0xae,track))+element(0x1f43b675,cluster))
def main():
    cases=[];invalid=[]
    for depth in [8,10,12,16]:
        for layout in ['gray','420','444','rgb']:
            for varied in [False,True]:
                w,h=17,9; chroma=layout!='gray';rgb=layout=='rgb';shift=(1,1) if layout=='420' else (0,0)
                rice=Rice();packets=[];frames=[];maximum=(1<<depth)-1
                for frame in range(2):
                    original=[((x*53+y*11+frame*17)&maximum) if (x+y)%5 else 0 for y in range(h) for x in range(w)]
                    if rgb:
                        pixels=[(v,(v*3+frame)&maximum,(maximum-v)) for v in original]
                        planes=[[],[],[]];bias=1<<depth
                        for red,green,blue in pixels:
                            base=blue if 9<=depth<=15 else green;cb=(green if 9<=depth<=15 else blue)-base;cr=red-base
                            for p,v in zip(planes,[base+((cb+cr)>>2),cb+bias,cr+bias]):p.append(v)
                        values=[v for pixel in pixels for v in pixel];sizes=[(w,h)]*3
                    else:
                        cw,ch=(w+(1<<shift[0])-1)>>shift[0],(h+(1<<shift[1])-1)>>shift[1]
                        planes=[original]+[[(i*71+component*23+frame)&maximum for i in range(cw*ch)] for component in range(2)] if chroma else [original]
                        sizes=[(w,h)]+[(cw,ch)]*2
                        values=[v for p in planes for v in p]
                        if not chroma:values += [1<<(depth-1)]*(2*w*h)
                    rice.bits='';trace_start=len(rice.trace);indexes=[0]*3
                    order=[(component,y) for y in range(h) for component in range(3)] if rgb else [(component,y) for component,p in enumerate(planes) for y in range(sizes[component][1])]
                    for component,y in order:
                        pw,_=sizes[component];bits=depth+int(rgb)
                        differences,contexts=coded_line(planes[component],pw,y,bits,varied)
                        indexes[component]=rice.line(differences,contexts,bits,int(component!=0),indexes[component])
                    used_bits=len(rice.bits);payload=rice.finish();head=header(depth,rgb,chroma,shift,varied,frame==0);packet=head+payload
                    name=f'ffv1-rice-{layout}-{depth}-contexts{int(varied)}-{frame}'
                    reference=bytes(values) if depth==8 else b''.join(v.to_bytes(2,'little') for v in values)
                    (D/(name+'.packet')).write_bytes(packet);(D/(name+'.samples')).write_bytes(reference)
                    frames.append(dict(packet=name+'.packet',reference=name+'.samples',header_bytes=len(head),rice_bytes=len(payload),trace=rice.trace[trace_start:],sha256=hashlib.sha256(packet).hexdigest()))
                    packets.append(packet)
                    if depth==8 and layout=='gray' and not varied and frame==0:
                        for label,bad,error in [('truncated',packet[:len(packet)//2],'truncated FFV1 Golomb Rice payload')]:
                            file=f'ffv1-rice-invalid-{label}.packet';(D/file).write_bytes(bad);invalid.append(dict(file=file,error=error,valid=name+'.packet'))
                        if used_bits%8:
                            bad=packet[:-1]+bytes([packet[-1]|1]);file='ffv1-rice-invalid-padding.packet';(D/file).write_bytes(bad)
                            invalid.append(dict(file=file,error='nonzero FFV1 Golomb Rice padding',valid=name+'.packet'))
                        esc=next(t for t in rice.trace[trace_start:] if t.get('escape'))
                        start=esc['offset']+12;bitstream=''.join(f'{v:08b}' for v in payload)
                        bitstream=bitstream[:start]+'0'*depth+bitstream[start+depth:]
                        bad=head+bytes(int(bitstream[i:i+8],2) for i in range(0,len(bitstream),8))
                        file='ffv1-rice-invalid-escape.packet';(D/file).write_bytes(bad)
                        invalid.append(dict(file=file,error='noncanonical FFV1 Golomb Rice escape',valid=name+'.packet'))
                file=f'ffv1-rice-{layout}-{depth}-contexts{int(varied)}.mkv';data=movie(packets,w,h,rgb);(D/file).write_bytes(data)
                cases.append(dict(file=file,depth=depth,layout=layout,size=[w,h],frames=frames,sha256=hashlib.sha256(data).hexdigest()))
    for source in list(cases):
        if source['depth']!=8:continue
        case=dict(source);case['frames']=[];packets=[]
        for index,frame in enumerate(source['frames']):
            old=(D/frame['packet']).read_bytes();rgb=source['layout']=='rgb';shift=(1,1) if source['layout']=='420' else (0,0)
            head=header(8,rgb,source['layout']!='gray',shift,'contexts1' in source['file'],index==0,version=0)
            packet=head+old[frame['header_bytes']:];name=frame['packet'].replace('ffv1-rice-','ffv1-rice-v0-')
            (D/name).write_bytes(packet);entry=dict(frame,packet=name,header_bytes=len(head),sha256=hashlib.sha256(packet).hexdigest());case['frames'].append(entry);packets.append(packet)
        case['file']=source['file'].replace('ffv1-rice-','ffv1-rice-v0-');data=movie(packets,*source['size'],source['layout']=='rgb');(D/case['file']).write_bytes(data)
        case['sha256']=hashlib.sha256(data).hexdigest();cases.append(case)
    for w,h in [(1,2),(64,4),(257,3)]:
        rice=Rice();index=0
        for y in range(h):index=rice.line([0]*w,[0]*w,8,0,index)
        head=header(8,False,False,(0,0),False,True);packet=head+rice.finish();name=f'ffv1-rice-zero-run-{w}x{h}'
        reference=bytes(w*h)+bytes([128])*(2*w*h)
        (D/(name+'.packet')).write_bytes(packet);(D/(name+'.samples')).write_bytes(reference)
        movie_data=movie([packet],w,h,False);(D/(name+'.mkv')).write_bytes(movie_data)
        cases.append(dict(file=name+'.mkv',depth=8,layout='gray',size=[w,h],sha256=hashlib.sha256(movie_data).hexdigest(),frames=[dict(packet=name+'.packet',reference=name+'.samples',header_bytes=len(head),rice_bytes=len(packet)-len(head),trace=rice.trace,sha256=hashlib.sha256(packet).hexdigest())]))
    (D/'ffv1-rice.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,provenance='Authored pixels, independent Python RFC 9043 Golomb Rice encoder, per-symbol context traces, run interruption and ESC; keyframe followed by adapted nonkeyframe. No private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
