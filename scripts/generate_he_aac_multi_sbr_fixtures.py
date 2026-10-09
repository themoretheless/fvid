#!/usr/bin/env python3
"""Original multi-element AAC/SBR PCE video regressions; independent mono DSP pointers."""
import json,hashlib
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def program(prefix,elements,top=False):
 groups=[[(kind,tag) for kind,tag,pos in elements if pos==p] for p in ['front','side','back','lfe']]
 bits=prefix+field(0,4)+field(1,2)+frequency(24000)+''.join(field(len(g),w) for g,w in zip(groups,[4,4,4,2]))+field(0,3)+field(0,4)+'000'
 for group,pos in zip(groups,['front','side','back','lfe']):
  for kind,tag in group:bits+=(field(kind==1,1) if pos!='lfe' else '')+field(tag,4)
 comment=b''
 if top:
  payload=bytes([0xac,0x40]);crc=0xff
  for byte in payload:
   crc^=byte
   for _ in range(8):crc=((crc<<1)^0x07)&255 if crc&128 else (crc<<1)&255
  comment=payload+bytes([crc])
 return bits+'0'*(-len(bits)%8)+field(len(comment),8)+''.join(field(b,8) for b in comment)

def config(slots,rate,kind,elements,indexed=False,top=False):
 ga=field(slots==15,1)+'00';channel_config=field(indexed if indexed else 0,4)
 prefix=(field(5,5)+frequency(24000)+channel_config+frequency(rate)+field(2,5)+ga) if kind=='explicit' else field(2,5)+frequency(24000)+channel_config+ga
 bits=prefix if indexed else program(prefix,elements,top)
 if kind=='sync':bits+=field(0x2b7,11)+field(5,5)+'1'+frequency(rate)
 return packed(bits)

def block(raw,kind,tag,fill=True):
 bits=''.join(f'{b:08b}' for b in raw);core=52 if kind==1 else 29
 count=int(bits[core+3:core+7],2);start=core+7
 if count==15:count=14+int(bits[start:start+8],2);start+=8
 end=start+count*8;assert bits[end:end+3]=='111'
 return bits[:3]+field(tag,4)+bits[7:(end if fill else core)]

def silent(kind,tag):return field(kind,3)+field(tag,4)+field(100,8)+'0'+'00'+'0'+'000000'+'0'+'000'

def main():
 missing=json.loads((DEST/'he-aac-missing-sbr.json').read_text())['cases']
 mono=json.loads((DEST/'he-aac-sbr-packets.json').read_text())['cases'];stereo=json.loads((DEST/'he-aac-sbr-stereo.json').read_text())['cases']
 mb=(DEST/'he-aac-sbr-packets.bin').read_bytes();sb=(DEST/'he-aac-sbr-stereo.bin').read_bytes();blob=bytearray();cases=[]
 for slots in [15,16]:
  for rate,bands in [(24000,32),(48000,64)]:
   for layout,elements,mapping in [('two-sce',[(0,3,'front'),(0,4,'back')],[0,1]),('height-two-sce',[(0,3,'front'),(0,4,'back')],[1,0]),('5.1',[(0,3,'front'),(1,5,'front'),(1,7,'back'),(3,9,'lfe')],[2,0,1,4,5,3]),('5.1-indexed',[(0,3,'front'),(1,5,'front'),(1,7,'back'),(3,9,'lfe')],[2,0,1,4,5,3]),('height-7.1-indexed',[(0,3,'front'),(1,5,'front'),(1,7,'back'),(3,9,'lfe'),(1,11,'top')],[2,0,1,4,5,3,6,7]),('5.1-missing',[(0,3,'front'),(1,5,'front'),(1,7,'back'),(3,9,'lfe')],[2,0,1,4,5,3])]:
    candidates=[c for c in mono if c['slots']==slots and c['bands']==bands and c['signalling']=='explicit'];pairs=[c for c in stereo if c['slots']==slots and c['bands']==bands and not c['coupled']]
    refs=[candidates[0],candidates[-1]] if layout.endswith('two-sce') else [candidates[-1],pairs[0],pairs[-1],None]
    if layout=='height-7.1-indexed':refs.append(pairs[0])
    for kind in ['explicit','sync','implicit']:
     if kind=='implicit' and bands==32:continue
     for reverse in [False,True]:
      if layout.endswith('indexed') and reverse:continue
      frames=[]
      for n in range(3):
       body=('' if layout.endswith('indexed') else program('101',elements,top=layout=='height-two-sce'));order=list(range(len(elements)))
       if reverse and n%2:order.reverse()
       for i in order:
        typ,tag,_=elements[i];ref=refs[i]
        if ref is None:body+=silent(typ,tag)
        else:
         raw=sb if typ==1 else mb;row=ref['frames'][n];src=raw[row['offset']:row['offset']+row['bytes']];body+=block(src,typ,tag,fill=not(layout=='5.1-missing' and i==2 and n==1))
       data=packed(body+'111');frames.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
      channels=sum(2 if typ==1 else 1 for typ,_,_ in elements)
      setup=config(slots,rate,kind,elements,indexed=14 if layout=='height-7.1-indexed' else (6 if layout=='5.1-indexed' else 0),top=layout=='height-two-sce');case=dict(slots=slots,bands=bands,frames=frames,asc=setup.hex(),pcm_offset=0,samples=slots*bands*6)
      video=video_fixture([case],blob,channels=channels,filename=f'he-aac-multi-sbr-{layout}-{kind}-{slots*64}-{rate}-{int(reverse)}-synthetic.mp4') if bands==64 else None
      missing_ref=next(r for r in missing if r['slots']==slots and r['bands']==bands and r['pattern']==[True,False,True]) if layout=='5.1-missing' else None
      cases.append(dict(case,channel_mask=0x503f if layout=='height-7.1-indexed' else (0x2100 if layout=='height-two-sce' else (0x104 if layout=='two-sce' else 0x3f)),missing_element=2 if missing_ref else None,missing_pcm_offset=missing_ref['pcm_offset'] if missing_ref else None,layout=layout,output_rate=rate,channels=channels,mapping=mapping,pcm_offsets=[r['pcm_offset'] if r else None for r in refs],widths=[2 if typ==1 else 1 for typ,_,_ in elements],video=video))
 invalid=[]
 seed=next(c for c in cases if c['layout']=='5.1' and c['slots']==16 and c['bands']==64)
 elements=[(0,3,'front'),(1,5,'front'),(1,7,'back'),(3,9,'lfe')]
 mono_ref=next(c for c in reversed(mono) if c['slots']==16 and c['bands']==64 and c['signalling']=='explicit')
 pair_ref=next(c for c in stereo if c['slots']==16 and c['bands']==64 and not c['coupled'])
 row=mono_ref['frames'][1];first=block(mb[row['offset']:row['offset']+row['bytes']],0,3)
 row=pair_ref['frames'][1];second=block(sb[row['offset']:row['offset']+row['bytes']],1,5)
 count=int(second[55:59],2);header=15 if count==15 else 7
 crc_at=len(program('101',elements))+len(first)+52+header+4
 badframes=[]
 for n,row in enumerate(seed['frames']):
  data=bytes(blob[row['offset']:row['offset']+row['bytes']])
  if n==1:
   data=bytearray(data);data[crc_at//8]^=1<<(7-crc_at%8);data=bytes(data)
  badframes.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
 bad=dict(seed,frames=badframes)
 bad['video']=video_fixture([bad],blob,channels=6,filename='he-aac-multi-sbr-second-crc-synthetic.mp4')
 bad['error']='SBR CRC mismatch';invalid.append(bad)
 (DEST/'he-aac-multi-sbr-packets.bin').write_bytes(blob)
 (DEST/'he-aac-multi-sbr-oracles.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,sha256=hashlib.sha256(blob).hexdigest()),indent=2)+'\n')
 print('multi SBR cases',len(cases))
if __name__=='__main__':main()
