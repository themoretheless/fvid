#!/usr/bin/env python3
"""Long/gapped/padded authored companion windows; offline finite EBML only."""
import json,struct,hashlib
from generate_hevc_matroska_timeline import fields,child,vint
from generate_hevc_matroska_formats import D
from generate_ffv1_gray_fixtures import element,uint

def main():
    refs=json.loads((D/'hevc-matroska-reorder.json').read_text())['cases'];cases=[]
    for depth in [8,10]:
        c=next(c for c in refs if c['depth']==depth and c['companions'] and not c['untimed'] and c['segment_end'])
        seg=child((D/c['file']).read_bytes(),0x18538067);tracks=child(seg,0x1654ae6b)
        audio={2:[],3:[]};video=[]
        for k,body in fields(seg):
            if k!=0x1f43b675:continue
            pts=int.from_bytes(child(body,0xe7),'big')
            for tag,value in fields(body):
                if tag not in [0xa3,0xa0]:continue
                block=value if tag==0xa3 else child(value,0xa1)
                number,at=vint(block,0,True);payload=block[at+3:]
                if number==1:video.append((pts,payload,block[at+2]))
                else:audio[number].append(payload)
        assert video and all(audio.values())
        for variant in ['long','gap','padding']:
            clusters=[]
            # Keep genuine I/P/B decode order; stretch only container timing.
            for pts,payload,flags in video:
                timestamp=pts*5 if variant=='long' else pts
                block=b'\x81\0\0'+bytes([flags])+payload
                clusters.append(element(0x1f43b675,uint(0xe7,timestamp)+element(0xa3,block)))
            for number,rate,frame in [(2,44100,1024),(3,48000,960)]:
                count=150 if variant=='long' else 12
                for i in range(count):
                    timestamp=(i*frame*1000000000+rate//2)//rate
                    if variant=='gap' and i>=3:timestamp+=37000000
                    block=bytes([128|number])+b'\0\0\x80'+audio[number][i%len(audio[number])]
                    if variant=='padding' and i in [3,5]:
                        pad=1000000 if i==3 else -1000000
                        payload=element(0xa0,element(0xa1,block)+element(0x75a2,pad.to_bytes(8,'big',signed=True)))
                    else:payload=element(0xa3,block)
                    clusters.append(element(0x1f43b675,uint(0xe7,timestamp)+payload))
            file=f'hevc-complex-audio-{depth}-{variant}.mkv'
            data=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,tracks)+b''.join(clusters))
            (D/file).write_bytes(data)
            interval=[1700123,1917789] if variant=='long' else [67123,199789]
            cases.append(dict(file=file,depth=depth,variant=variant,interval_us=interval,frames=c['frames'],size=c['size'],sha256=hashlib.sha256(data).hexdigest()))
    (D/'hevc-complex-audio-windows.json').write_text(json.dumps(dict(cases=cases,provenance='Own saved I/P/B access units and synthetic AAC/Opus packets repeated as valid coded frames, with authored nanosecond timing, 37 ms gaps and +/-1 ms interior padding. No private data, codec execution, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
