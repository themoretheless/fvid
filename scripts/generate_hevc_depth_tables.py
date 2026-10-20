"""Authored DLT syntax and literal tables. Offline component fixtures."""
import json
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/hevc-depth-tables'
def field(v,n):return f'{v:0{n}b}'
def header(depth,count):return '1'+field(count-1,6)+field(depth-8,4)
def delta(depth,items):
    bits=field(len(items),depth)
    if not items:return bits
    gaps=[b-a for a,b in zip(items,items[1:])];maximum=max(gaps,default=0)
    if len(items)>1:bits+=field(maximum,depth)
    minimum=min(gaps,default=0)
    if len(items)>2 and maximum:bits+=field(minimum-1,maximum.bit_length())
    bits+=field(items[0],depth)
    if maximum>minimum:
        bits+=''.join(field(gap-minimum,(maximum-minimum).bit_length())for gap in gaps)
    return bits

def generate():
    cases=[];files={}
    def add(name,bits,depth,ids,expected=None,error=None):
        payload=bits+'0'*(-len(bits)%8)
        files[name+'.bin']=int(payload,2).to_bytes(len(payload)//8,'big')
        cases.append(dict(name=name,bits=len(bits),depth=depth,ids=ids,expected=expected,error=error))
    add('absent','0',8,[],None)
    add('disabled',header(8,3)+'000',8,[0,7,62],[None,None,None])
    for depth in [8,10,12,16]:
        vals=[0,1,(1<<(depth-1)),(1<<depth)-1]
        add(f'flags-{depth}',header(depth,1)+'101'+''.join('1'if v in vals else'0'for v in range(1<<depth)),depth,[3],[vals])
        add(f'delta-{depth}',header(depth,1)+'100'+delta(depth,vals),depth,[3],[vals])
    add('flags-full-8',header(8,1)+'101'+'1'*256,8,[3],[list(range(256))])
    for name,items in [('empty',[]),('singleton',[200]),('pair',[4,9]),('variable',[1,2,4,9]),('duplicates',[7,7,7])]:
        add(name,header(8,1)+'100'+delta(8,items),8,[7],[list(dict.fromkeys(items))])
    first=[0,3,128,255]
    flags=''.join('1'if v in first else'0'for v in range(256))
    add('predicted-xor',header(8,2)+'101'+flags+'11'+delta(8,[3,4,255]),8,[3,7],[first,[0,4,128]])
    add('predicted-copy',header(8,2)+'101'+flags+'11'+delta(8,[]),8,[3,7],[first,first])
    add('bad-first-prediction',header(8,1)+'11'+delta(8,[]),8,[3],error='HEVC DLT prediction lacks a first-layer table')
    add('bad-missing-reference',header(8,2)+'0'+'11'+delta(8,[]),8,[3,7],error='HEVC DLT prediction lacks a first-layer table')
    add('bad-depth',header(8,1)+'0',10,[3],error='HEVC DLT depth disagrees with SPS')
    add('bad-count',header(8,2)+'00',8,[3],error='HEVC DLT layer count disagrees with VPS')
    add('bad-overflow',header(8,1)+'100'+field(2,8)+field(5,8)+field(253,8),8,[3],error='HEVC DLT value exceeds depth range')
    add('bad-minimum',header(8,1)+'100'+field(3,8)+field(3,8)+field(3,2),8,[3],error='HEVC DLT minimum difference exceeds maximum')
    add('bad-gap',header(8,1)+'100'+field(3,8)+field(5,8)+field(0,3)+field(0,8)+field(7,3),8,[3],error='HEVC DLT difference exceeds maximum')
    files['manifest.json']=(json.dumps(dict(provenance='Own Annex I field encoder and literal tables. Component acceptance only; active 3D playback is not connected.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
