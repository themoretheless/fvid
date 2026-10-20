"""Authored full-width adaptive ALAC and neutral VP9. No external encoder."""
import json,struct
from pathlib import Path
import generate_alac_multichannel_fixtures as g
from generate_alac_cookie_wrappers import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-full-width-adaptive'
def generate():
    files={};cases=[]
    cookie=bytearray(24);cookie[:4]=g.word(128)
    cookie[5:10]=bytes([32,40,10,14,1]);cookie[20:24]=g.word(48000)
    for order in [1,2,4,8,16,30]:
        for sign in [-1,1]:
            name=f'order{order}-'+('negative'if sign<0 else'positive')
            values=[sign*(0x40000001+n*7+(23 if n>=80 else 0))for n in range(256)]
            packets=[];stats=dict(ordinary=0,escapes=0,runs=0,run_samples=0,resumed_run_samples=0)
            for start in [0,128]:
                residual=g.adaptive_residuals(values[start:start+128],order,stats,quant=1)
                bits='000'+'0000'+'0'*12+'1'+'00'+'0'+f'{128:032b}'
                bits+='0'*16+'0000'+'0001'+'100'+f'{order:05b}'+'0'*(order*16)
                bits+=g.rice_plane(residual,32,stats)
                bits+='111';bits+='0'*(-len(bits)%8)
                packets.append(int(bits,2).to_bytes(len(bits)//8,'big'))
            assert stats['adaptive_updates']>0
            files[name+'.mkv']=movie(cookie,1,packets)
            files[name+'.m4a']=g.mp4(cookie,1,32,packets)
            files[name+'.i32le']=struct.pack('<256i',*values)
            files[name+'.cookie']=bytes(cookie)
            for i,p in enumerate(packets):files[name+f'.packet{i}']=p
            cases.append(dict(name=name,order=order,sign=sign,stats=stats))
    files['manifest.json']=(json.dumps(dict(provenance='Authored literal full-width PCM and own packets/neutral VP9; offline only',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
