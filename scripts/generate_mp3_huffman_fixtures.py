#!/usr/bin/env python3
"""Offline signed Huffman corpus from independent standard codeword data.
Exercises every pair/quad codeword, escape endpoints and all sign patterns.
"""
import json,struct,itertools,hashlib
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp3-huffman'
books={b['id']:b['entries']for b in json.loads((D/'codewords.json').read_text())['books']}
records=[]
def add(kind,table,word,expected):
 prefix=len(records)%8;bits=[i%2 for i in range(prefix)]+word+[1,0,1,1,0,0,1,1];bits += [0]*(-len(bits)%8)
 payload=bytes(sum(bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(bits),8))
 records.append(struct.pack('>BBBH',kind,table,prefix,len(word))+bytes([len(payload)])+struct.pack('<4h',*(expected+[0]*(4-len(expected))))+payload)
def bits(value,n):return [(value>>i)&1 for i in range(n-1,-1,-1)]
add(0,0,[],[0,0])
for table in range(1,32):
 if table in [4,14]:continue
 tree=16 if 16<=table<=23 else 24 if table>=24 else table
 linbits=[1,2,3,4,6,8,10,13][table-16]if 16<=table<=23 else[4,5,6,7,8,9,11,13][table-24]if table>=24 else 0
 for code,length,symbol in books[tree]:
  base=[symbol>>4,symbol&15]
  extensions=[sorted(set([0,1,(1<<linbits)-1]))if linbits and v==15 else[0]for v in base]
  for extra in itertools.product(*extensions):
   absolute=[v+e for v,e in zip(base,extra)];positions=[i for i,v in enumerate(absolute)if v]
   for signs in itertools.product([0,1],repeat=len(positions)):
    negative=dict(zip(positions,signs));word=bits(code,length);expected=[]
    for i,value in enumerate(absolute):
     if linbits and base[i]==15:word+=bits(extra[i],linbits)
     if value:word.append(negative[i])
     expected.append(-value if negative.get(i,0)else value)
    add(0,table,word,expected)
for table in [0,1]:
 for code,length,symbol in books[32+table]:
  base=[(symbol>>(3-i))&1 for i in range(4)];positions=[i for i,v in enumerate(base)if v]
  for signs in itertools.product([0,1],repeat=len(positions)):
   negative=dict(zip(positions,signs));word=bits(code,length)+list(signs)
   add(1,table,word,[-v if negative.get(i,0)else v for i,v in enumerate(base)])
output=b'FVMP3H1\0'+struct.pack('>I',len(records))+b''.join(records)
(D/'signed-corpus.bin').write_bytes(output)
(D/'manifest.json').write_text(json.dumps(dict(records=len(records),sha256=hashlib.sha256(output).hexdigest(),scope='Entropy symbols and bit consumption; not compressed PCM acceptance'),indent=2)+'\n')
print(len(records),'signed MP3 Huffman vectors')
# Granule entropy plans: regions, zero tables, count1, stuffing and 576-line cap.
def pair_word(table,values):
 if table==0:
  assert values==[0,0];return []
 tree=16 if 16<=table<=23 else 24 if table>=24 else table
 lin=[1,2,3,4,6,8,10,13][table-16]if 16<=table<=23 else[4,5,6,7,8,9,11,13][table-24]if table>=24 else 0
 absolute=list(map(abs,values));base=[min(v,15)if lin else v for v in absolute]
 code,length,_=next(e for e in books[tree]if e[2]==base[0]*16+base[1]);word=bits(code,length)
 for i,v in enumerate(absolute):
  if lin and base[i]==15:word+=bits(v-15,lin)
  if v:word.append(int(values[i]<0))
 return word
def quad_word(table,values):
 symbol=sum(int(v!=0)<<(3-i)for i,v in enumerate(values));code,length,_=next(e for e in books[32+table]if e[2]==symbol)
 return bits(code,length)+[int(v<0)for v in values if v]
plans=[
 ([],[0,0],[0,0,0],0,[],[0]),
 ([[0,0]]*288,[0,576],[0,0,0],1,[],[1,1,0]),
 ([[-1,1],[2,-2],[16,-15]],[2,4],[1,2,16],0,[[1,-1,0,1],[-1,0,-1,0]],[0]),
 ([[8206,-8206],[-15,16]],[0,0],[1,2,31],1,[[1,0,-1,1]],[1,0]),
 ([[1,-1]]*288,[72,288],[1,2,24],0,[],[1,0]),
 ([[1,0],[-1,1]],[576,576],[3,0,0],1,[[1,0,-1,0]]*143,[1,1,1,1]),
]
encoded=[]
for number,(pairs,ends,tables,count1,quads,stuff)in enumerate(plans):
 word=[];expected=[]
 for i,pair in enumerate(pairs):
  region=0 if i*2<ends[0]else 1 if i*2<ends[1]else 2
  word+=pair_word(tables[region],pair);expected+=pair
 big_bits=len(word)
 for quad in quads:word+=quad_word(count1,quad);expected+=quad
 huffman_bits=len(word);word+=stuff;nonzero=max([i+1 for i,v in enumerate(expected)if v]+[0]);expected+=[0]*(576-len(expected));assert len(expected)==576
 prefix=number%8;packed=[1]*prefix+word+[1]*8;packed+=[0]*(-len(packed)%8)
 payload=bytes(sum(packed[i+j]<<(7-j)for j in range(8))for i in range(0,len(packed),8))
 record=struct.pack('>3H',len(pairs),*ends)+bytes(tables+[count1,prefix])+struct.pack('>5H',len(word),len(payload),huffman_bits,big_bits,nonzero)
 encoded.append(record+struct.pack('<576h',*expected)+payload)
spectrum=b'FVMP3S1\0'+struct.pack('>I',len(encoded))+b''.join(encoded)
(D/'spectrum-cases.bin').write_bytes(spectrum)
manifest=json.loads((D/'manifest.json').read_text());manifest.update(spectrum_records=len(encoded),spectrum_sha256=hashlib.sha256(spectrum).hexdigest());(D/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
