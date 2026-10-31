#!/usr/bin/env python3
"""Own residual ICC/ICS stimuli. Separate generation; no foreign encoder/network."""
import json
from pathlib import Path
from generate_aac_main_tools_fixtures import channel,ics,tuple_bits
from generate_aac_ld_mps_config_fixtures import RATES,TREES,length
from generate_aac_ld_mps_transport_fixtures import packed,field,envelope
from generate_aac_ld_mps_inband_fixtures import raw
from generate_he_aac_packet_fixtures import video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-residual'
def difference(v,tail):return '0'*v+'1' if v<7 else '0000000'+str(tail)
def block(seq,shape,seed,rate_index):
 width=4 if seq==2 else [4,4,4,4,4,4,4,4,8,8,8,12,12][rate_index]
 values=[[(seed+w+i)%3-1 for i in range(width)]for w in range(8 if seq==2 else 1)]
 wire=channel(seq,[1],values,info=ics(seq,1,False,shape=shape))
 if seq!=2:wire+=''.join(tuple_bits(values[0][i:i+4])for i in range(4,width,4))
 sparse=[[w*128+i if seq==2 else i, v*1024]for w,row in enumerate(values)for i,v in enumerate(row)if v]
 return wire,dict(sequence=seq,shape=shape,sparse=sparse)
def payload(tree,rate_index,seq,slots,frames_code,bands,bad_extra=False):
 wire='';boxes=[];frames=frames_code+1;update=slots//frames
 for box,band in enumerate(bands):
  if not band:boxes.append(None);continue
  differences=[]
  if box<len(TREES[tree][2]):
   for ps in range(2):
    active=ps==0 or(box+tree)%2==0;wire+=str(int(active));values=[(rate_index+box+ps+b)%8 for b in range(band)]if active else None
    if active:wire+=''.join(difference(v,(ps+box)%2)for v in values)
    differences.append(values)
  blocks=[]
  for f in range(frames):
   bits,first=block(seq,(f+box)%2,f+box+rate_index,rate_index);wire+=bits;extra=None
   if seq==2 and update in(18,24,30):
    bits,extra=block(0 if bad_extra else seq,(f+box+1)%2,f+box+rate_index+1,rate_index);wire+=bits
   blocks.append(dict(first=first,extra=extra))
  boxes.append(dict(icc=differences,blocks=blocks))
 return wire,boxes

def main():
 OUT.mkdir(exist_ok=True);blob=bytearray();cases=[]
 for tree in range(8):
  for ri,rate in enumerate(RATES):
   for seq in range(4):
    fc=(tree+ri+seq)%4;slots=(18,24,30)[ri%3]if seq==2 else 32;bands=[None if (b+tree+ri)%3==1 else 0 if(b+ri)%5==0 else b%3+1 for b in range(len(TREES[tree][2])+TREES[tree][3])]
    if not any(bands):bands[0]=1
    bits,boxes=payload(tree,ri,seq,slots,fc,bands);data=packed(bits+'10100101')
    cases.append(dict(tree=tree,rate=rate,seq=seq,slots=slots,frames_code=fc,bands=bands,offset=len(blob),bytes=len(data),bits=len(bits),boxes=boxes));blob.extend(data)
 # An extra short ICS must agree with the first window sequence.
 bits,_=payload(7,3,2,18,0,[1],True);bad=dict(tree=7,rate=48000,slots=18,frames_code=0,bands=[1],offset=len(blob),bytes=len(packed(bits)),bits=len(bits));blob.extend(packed(bits))
 (OUT/'payloads.bin').write_bytes(blob)
 # A complete owned movie exposes the forbidden LD residual profile.
 parent=FIX/'aac-ld-mps-stp';base=next(c for c in json.loads((parent/'manifest.json').read_text())['cases']if c['name']=='n512-c1-base-x2-crc1-bands23-on');source=(parent/'packets.bin').read_bytes()
 ssc=bytes.fromhex(base['ordinary_ssc'])+packed('0000'+length(2)+'0011001000010000')
 original=bytes.fromhex(base['ssc']);asc_bits=raw(bytes.fromhex(base['asc']));suffix='0010'+length(len(original))+raw(original)+'0000'+'00';at=asc_bits.rfind(suffix);assert at>=0
 asc=packed(asc_bits[:at]+'0010'+length(len(ssc))+raw(ssc)+'0000'+'00');packets=bytearray();frames=[]
 for i,row in enumerate(base['frames']):
  prefix=raw(source[row['offset']:row['offset']+row['bytes']])[:row['prefix_bits']]
  residual,_=payload(7,3,0,base['slots'],0,[1]);data=packed(residual);body=bytes.fromhex(row['ordinary'])+bytes([len(data)])+data
  current=bytes([len(ssc)])+ssc+body if i==0 else body
  packet=packed(prefix+envelope(9,1 if i==0 else 0,3,current,True));frames.append(dict(offset=len(packets),bytes=len(packet),prefix_bits=len(prefix),spatial=body.hex()));packets.extend(packet)
 movie=dict(base,asc=asc.hex(),frames=frames);(OUT/'native-residual-refusal.mp4').write_bytes(video_fixture([movie],packets,1,write=False)['data']);(OUT/'native-packets.bin').write_bytes(packets);(OUT/'native.asc').write_bytes(asc);(OUT/'native.ssc').write_bytes(ssc)
 (OUT/'manifest.json').write_text(json.dumps(dict(scope='Residual syntax/spectral acceptance only; native LD residual coding is forbidden by the profile.',cases=cases,extra_mismatch=bad,native=dict(frames=frames)),separators=(',',':'))+'\n');print(len(cases),'residual syntax cases; one explicit native-refusal movie')
if __name__=='__main__':main()
