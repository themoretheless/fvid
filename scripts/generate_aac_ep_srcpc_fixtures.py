"""Own EP channel-code vectors and short protected ELD video; explicit/offline.

Reference encoder uses a polynomial impulse sequence, not production registers
or a Viterbi trellis. The EP video is an acceptance source for the owned EP2 raw-AU route.
"""
import json,struct
from pathlib import Path
from generate_aac_eld_fixtures import generate as eld
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aac-ep-srcpc'
# ISO/IEC 14496-3:2009 Table 1.61 numeric puncture facts, explicit rows.
MASKS=[(255,a,b,c)for a,b,c in [
(0,0,0),(128,0,0),(136,0,0),(168,0,0),(170,0,0),(234,0,0),(238,0,0),(254,0,0),(255,0,0),
(255,128,0),(255,136,0),(255,168,0),(255,170,0),(255,234,0),(255,238,0),(255,254,0),(255,255,0),
(255,255,128),(255,255,136),(255,255,168),(255,255,170),(255,255,234),(255,255,238),(255,255,254),(255,255,255)]]
TAILS=[0,13,10,7,4,9,14,3,8,5,2,15,12,1,6,11]
POWERS={0:[],1:[1,0],2:[2,1,0],3:[3,1,0],4:[4,3,2,0],5:[5,4,2,0],6:[6,5,3,2,1,0],7:[7,6,2,0],8:[8,2,1,0],9:[9,8,5,2,1,0],10:[10,9,5,4,1,0],11:[11,10,4,3,1,0],12:[12,11,3,2,1,0],13:[13,12,7,6,5,4,2,0],14:[14,13,5,3,2,0],15:[15,14,11,10,7,6,2,0],16:[16,12,5,0],24:[24,23,6,5,1,0],32:[32,26,23,22,16,12,11,10,8,7,5,4,2,1,0]}
def code(message,rate,terminate):
    # Solve (1+D+D^2+D^4)c=u, then convolve c with output polynomials.
    c=[];u=[int(v)for v in message];n=len(u)
    at=lambda i:c[i]if i>=0 else 0
    for t in range(n+4*terminate):
        feedback=at(t-1)^at(t-2)^at(t-4)
        if t==n:state=sum(at(t-i-1)<<i for i in range(4))
        if t>=n:u.append(feedback)
        c.append(u[t]^feedback)
    result=''
    for t in range(len(c)):
        columns=[u[t],at(t)^at(t-3)^at(t-4),at(t)^at(t-2)^at(t-3)^at(t-4),at(t)^at(t-1)^at(t-3)^at(t-4)]
        result+=''.join(str(v)for v,mask in zip(columns,MASKS[rate])if mask&(128>>(t%8)))
    if terminate:
        assert int(''.join(map(str,u[-4:])),2)==TAILS[state]
    return result

def generate():
    files={};vectors=[];crc=[]
    powers=POWERS
    for n in [0,1,7,8,31,64]:
        message=''.join(str((i//3+i//7)%2)for i in range(n))
        for width,exponents in powers.items():
            remainder=int(message or '0',2)<<width;divisor=sum(1<<v for v in exponents)
            if width:
                while remainder.bit_length()>width:remainder^=divisor<<(remainder.bit_length()-width-1)
                output=f'{remainder^((1<<width)-1):0{width}b}'
            else:output=''
            crc.append(dict(message=message,width=width,parity=output))
    for n in [0,1,4,7,8,9,15,16,33,64]:
        message=''.join(str(((i*i+7*i+11)//3)%2)for i in range(n))
        for rate in range(25):
            for terminated in [False,True]:
                vectors.append(dict(message=message,rate=rate,terminated=terminated,encoded=code(message,rate,terminated)))
    # Preserve all 16 normative tail rows independently of the encoder oracle.
    tails=[]
    for state,expected in enumerate(TAILS):
        c=[int(v)for v in f'{state:04b}'];prefix=''
        for t in range(4):prefix+=str(c[t]^(c[t-1]if t>=1 else 0)^(c[t-2]if t>=2 else 0)^(c[t-4]if t>=4 else 0))
        tails.append(dict(state=state,prefix=prefix,tail=f'{expected:04b}'))
    base=eld(True,[(512,24000,1,'base',0,0,False)]);case=json.loads(base['manifest.json'])['cases'][0]
    # EP2: one set, no interleaving/stuffing, one frame, one class until AU end;
    # SRCPC 8/32, terminated, inverted CRC16, no reordering, basic (empty) header.
    ep='00000001'+'00'+'000'+'001'+'000001'+'100'+'00'+'1'+'0'+'0000'+'11000'+'10000'+'0'+'0'
    asc=packed(field(31,5)+field(7,6)+frequency(24000)+field(1,4)+'0'+'000'+'0'+'0000'+'10'+ep)
    blob=bytearray();frames=[]
    for row in case['frames']:
        a=row['offset'];raw=base['packets.bin'][a:a+row['bytes']]
        message=''.join(f'{v:08b}'for v in raw);payload_bits=len(message)
        remainder=int(message,2)<<16;divisor=sum(1<<v for v in powers[16])
        while remainder.bit_length()>16:remainder^=divisor<<(remainder.bit_length()-17)
        message+=f'{remainder^65535:016b}'
        protected=code(message,24,True);assert len(protected)%8==0
        packet=packed(protected);frames.append(dict(offset=len(blob),bytes=len(packet),information_bits=len(message),payload_bits=payload_bits,crc_width=16));blob.extend(packet)
    protected_case=dict(case,asc=asc.hex(),frames=frames)
    files['protected.mp4']=video_fixture([protected_case],blob,1,filename='protected.mp4',write=False)['data']
    files['plain.mp4']=base[case['video']];files['plain.asc']=base[case['name']+'.asc'];files['protected.asc']=asc
    files['pcm.f32le']=base[case['name']+'.f32le'];files['packets.bin']=bytes(blob)
    files['manifest.json']=(json.dumps(dict(vectors=vectors,tails=tails,crc=crc,frames=frames,provenance='Own polynomial impulse-sequence encoder and normative Tables 1.60/1.61; own ELD core/cosine PCM and own MP4 video; no private media, external process/codec or network.',scope='SRCPC primitive and EP2 single-class raw-AU acceptance; separate ESC mappings, RS, interleaving and concatenated classes remain separate gaps.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
