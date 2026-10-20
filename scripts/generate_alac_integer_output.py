"""Own full-width ALAC PCM and neutral synthetic VP9; offline generation."""
from pathlib import Path
import struct
import generate_alac_multichannel_fixtures as g
from generate_alac_cookie_wrappers import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-integer-output'
def generate():
    values=[0x40000001+i for i in range(8)]+[-0x40000001-i for i in range(8)]
    values += [-2147483648,2147483647,0,-1]+list(range(12))
    cookie=bytearray(24);cookie[:4]=g.word(16)
    cookie[5:10]=bytes([32,40,10,14,1]);cookie[20:24]=g.word(48000)
    packets=[]
    for start in (0,16):
        bits='000'+'0000'+'0'*12+'1'+'00'+'1'+f'{16:032b}'
        bits+=''.join(f'{v&0xffffffff:032b}' for v in values[start:start+16])
        bits+='111';bits+='0'*(-len(bits)%8)
        packets.append(int(bits,2).to_bytes(len(bits)//8,'big'))
    compressed=[]
    for start in (0,16):
        bits='000'+'0000'+'0'*12+'1'+'00'+'0'+f'{16:032b}'
        bits+='0'*16+'0000'+'1001'+'100'+'00000'
        stats=dict(ordinary=0,escapes=0,runs=0,run_samples=0,resumed_run_samples=0)
        bits+=g.rice_plane(values[start:start+16],32,stats)
        bits+='111';bits+='0'*(-len(bits)%8)
        compressed.append(int(bits,2).to_bytes(len(bits)//8,'big'))
    predictive=[]
    ramp=[0x40000001+i for i in range(16)]+[-0x40000001-i for i in range(16)]
    for start in (0,16):
        bits='000'+'0000'+'0'*12+'1'+'00'+'0'+f'{16:032b}'
        # q9, order1, coefficient1024 predicts base + twice the delta.
        bits+='0'*16+'0000'+'1001'+'100'+'00001'+f'{1024:016b}'
        residual=[ramp[start],ramp[start+1]-ramp[start]]+[0]*14
        stats=dict(ordinary=0,escapes=0,runs=0,run_samples=0,resumed_run_samples=0)
        bits+=g.rice_plane(residual,32,stats)
        bits+='111';bits+='0'*(-len(bits)%8)
        predictive.append(int(bits,2).to_bytes(len(bits)//8,'big'))
    return {'predictor.mkv':movie(cookie,1,predictive),
            'predictor.m4a':g.mp4(cookie,1,32,predictive),
            'predictor.i32le':struct.pack('<32i',*ramp),
            'rice.mkv':movie(cookie,1,compressed),
            'rice.m4a':g.mp4(cookie,1,32,compressed),
            'control.mkv':movie(cookie,1,packets),
            'control.m4a':g.mp4(cookie,1,32,packets),
            'expected.i32le':struct.pack('<32i',*values)}
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
