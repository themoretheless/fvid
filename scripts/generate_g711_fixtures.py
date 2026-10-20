#!/usr/bin/env python3
"""Authored all-code G.711 audio and three-frame raw RGB AVI reproducers.
Pure Python; no codec backend, encoder executable, private media or network.
"""
import hashlib,json,struct
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def chunk(tag,data,big=False):return tag+len(data).to_bytes(4,'big'if big else'little')+data+bytes(len(data)&1)
def lst(tag,data):return chunk(b'LIST',tag+data)
def level(law,code):
    # Independent segment reconstruction levels, not the decoder's shifts.
    q=code^ (0x55 if law=='alaw' else 0xff);segment=(q>>4)&7;mantissa=q&15
    offsets=[8,264,528,1056,2112,4224,8448,16896]if law=='alaw'else[0,132,396,924,1980,4092,8316,16764]
    steps=[16,16,32,64,128,256,512,1024]if law=='alaw'else[8,16,32,64,128,256,512,1024]
    value=offsets[segment]+steps[segment]*mantissa
    return value if code&128 else -value

def header(kind,handler,scale,rate,length,sample_size,rectangle=(0,0,0,0)):
    return struct.pack('<4s4sIHHIIIIIIIIhhhh',kind,handler,0,0,0,0,scale,rate,0,length,32768,0xffffffff,sample_size,*rectangle)
def main():
    items=[]
    for law,tag,au in [('alaw',6,27),('mulaw',7,1)]:
        table=b''.join(struct.pack('<h',level(law,c))for c in range(256));(D/f'g711-{law}-codes.s16le').write_bytes(table)
        for channels in [1,2]:
            name=f'g711-{law}-{channels}ch';frames=1024
            codes=bytes(c for frame in range(frames)for c in ([frame%256]if channels==1 else[frame%256,255-frame%256]))
            expected=b''.join(struct.pack('<h',level(law,c))for c in codes);(D/f'{name}.s16le').write_bytes(expected)
            fmt=struct.pack('<HHIIHHH',tag,channels,8000,8000*channels,channels,8,0)
            info=chunk(b'LIST',b'INFO'+chunk(b'INAM',b'Owned G711\0'))
            body=b'WAVE'+chunk(b'fmt ',fmt)+chunk(b'fact',struct.pack('<I',frames))+info+chunk(b'data',codes)
            wave=b'RIFF'+struct.pack('<I',len(body))+body;(D/f'{name}.wav').write_bytes(wave)
            snd=b'.snd'+struct.pack('>IIIII',24,len(codes),au,8000,channels)+codes;(D/f'{name}.au').write_bytes(snd)
            comm=struct.pack('>HIH',channels,frames,16)+struct.pack('>HQ',0x400b,8000<<51)+(b'alaw'if law=='alaw'else b'ulaw')+b'\x04G711\0'
            form=b'AIFC'+chunk(b'FVER',struct.pack('>I',0xa2805140),True)+chunk(b'COMM',comm,True)+chunk(b'SSND',bytes(8)+codes,True)
            aifc=b'FORM'+struct.pack('>I',len(form))+form;(D/f'{name}.aifc').write_bytes(aifc)
            upper=aifc.replace(b'alaw'if law=='alaw'else b'ulaw',b'ALAW'if law=='alaw'else b'ULAW',1)
            (D/f'{name}-uppercase.aifc').write_bytes(upper)
            avih=struct.pack('<14I',40000,400000,0,0x10,3,0,2,32768,64,64,0,0,0,0)
            video_fmt=struct.pack('<IiiHHIIIIII',40,64,64,1,24,0,64*64*3,0,0,0,0)
            hdrl=chunk(b'avih',avih)+lst(b'strl',chunk(b'strh',header(b'vids',b'DIB ',1,25,3,0,(0,0,64,64)))+chunk(b'strf',video_fmt))
            hdrl+=lst(b'strl',chunk(b'strh',header(b'auds',bytes(4),1,8000,frames,channels))+chunk(b'strf',fmt))
            movi=b'';index=b'';offset=4;audio_at=0
            for frame,count in enumerate([128,257,639]):
                image=bytes([40+frame*60,20+frame*40,200-frame*50])*(64*64)
                for record_id,payload,flags in [(b'00db',image,0x10),(b'01wb',codes[audio_at*channels:(audio_at+count)*channels],0)]:
                    record=chunk(record_id,payload);index+=struct.pack('<4sIII',record_id,flags,offset,len(payload));movi+=record;offset+=len(record)
                audio_at+=count
            body=b'AVI '+lst(b'hdrl',hdrl)+lst(b'movi',movi)+chunk(b'idx1',index)
            avi=b'RIFF'+struct.pack('<I',len(body))+body;(D/f'{name}.avi').write_bytes(avi)
            files=[f'{name}.{ext}'for ext in ['wav','avi','au','aifc','s16le']]+[f'{name}-uppercase.aifc']
            items.append(dict(law=law,channels=channels,sample_rate=8000,sample_frames=frames,avi_records=[128,257,639],files=[dict(file=f,sha256=hashlib.sha256((D/f).read_bytes()).hexdigest())for f in files]))
    (D/'g711-native.json').write_text(json.dumps(dict(generator=__file__,reference='https://www.rfc-editor.org/rfc/rfc3551.html#section-4.5.14',fixtures=items),indent=2)+'\n')
if __name__=='__main__':main()
