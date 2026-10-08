#!/usr/bin/env python3
"""Original AAC-LC height PCE + direct-cosine PCM oracle. Explicit generation."""
from pathlib import Path
import hashlib, json, math, re, struct
root=Path(__file__).resolve().parents[1]; folder=root/'tests/fixtures/playback-errors'
tables=(root/'crates/fvid-media/src/owned_aac/aac_huffman_tables.rs').read_text()
def table(name):
    body=re.search(r'const '+name+r':[^=]+ = \[(.*?)\];',tables,re.S)[1]
    return [int(n,0) for n in re.findall(r'0x[0-9a-fA-F]+|\d+',body)]
codes=table('SPECTRUM_CODEBOOK1_CODES'); lengths=table('SPECTRUM_CODEBOOK1_LENS')
sc=table('SCF_CODEBOOK_CODES'); sl=table('SCF_CODEBOOK_LENS')
# position, pair, tag, height. Decoder order is deliberately not inferred from
# the CRC/header parser: the expected per-channel order is explicitly authored.
cases={
 'mono-top': ([('front',False,0,1)], [0],1<<13),
 'mono-bottom': ([('front',False,0,2)], [0],0),
 'layered-front': ([('front',False,0,1),('front',False,1,2),('front',False,2,0)], [2,0,1],0),
 'layered-front-swap': ([('front',False,0,2),('front',False,1,1),('front',False,2,0)], [2,1,0],0),
 'top-front-pair': ([('front',False,0,0),('front',True,0,1)], [0,1,2],(1<<2)|(1<<12)|(1<<14)),
 'top-front-three': ([('front',False,0,1),('front',True,0,1)], [1,0,2],(1<<12)|(1<<13)|(1<<14)),
 'top-back-three': ([('back',False,0,1),('back',True,0,1)], [1,0,2],(1<<15)|(1<<16)|(1<<17)),
 'top-side': ([('front',False,0,0),('side',True,0,1)], [0,1,2],0),
 'bottom-surround': ([('front',False,0,2),('side',True,0,2),('back',True,1,2),('lfe',False,0,0)], [5,0,1,2,3,4],0),
 'top-wide': ([('front',False,0,1),('front',True,0,1),('front',True,1,1)], [0,1,2,3,4],0),
 'top-with-lfe': ([('front',False,0,0),('front',False,1,1),('back',False,2,1),('lfe',False,0,0)], [0,3,1,2],(1<<2)|(1<<3)|(1<<13)|(1<<16)),
 'normal-four': ([('front',True,0,0),('front',True,1,0)], [0,1,2,3],0),
 'multiple-lfe': ([('front',False,0,0),('lfe',False,0,0),('lfe',False,1,0)], [0,1,2],0),
 'normal-three-sce': ([('front',False,0,0),('front',False,1,0),('front',False,2,0)], [0,1,2],0),
 'normal-crc': ([('front',False,0,0)], [0],1<<2),
 'short-comment': ([('front',False,0,0)], [0],1<<2),
 'short-comment-two': ([('front',False,0,0)], [0],1<<2),
}
def crc(data):
    value=255
    for byte in data:
        for shift in range(7,-1,-1):
            feedback=((value>>7)^((byte>>shift)&1))&1
            value=((value<<1)&255)^(7 if feedback else 0)
    return value
manifest={'cases':{}}
for name,(elements,order,mask) in cases.items():
    flags=''.join(f'{height:02b}' for pos,pair,tag,height in elements if pos!='lfe')
    flags+='0'*(-len(flags)%8)
    comment=bytes([0xac])+int(flags,2).to_bytes(len(flags)//8,'big')
    comment+=bytes([crc(comment)])
    if name=='short-comment': comment=b'\xac'
    if name=='short-comment-two': comment=b'\xac\x55'
    output=bytearray(); pcm=bytearray(); channels=len(order); overlaps=[[0.0]*1024 for _ in order]
    for frame in range(6):
        fields=[(5,3),(0,4),(1,2),(3,4)]
        fields += [(sum(p==pos for p,_,_,_ in elements),width) for pos,width in [('front',4),('side',4),('back',4),('lfe',2)]]
        fields += [(0,3),(0,4),(0,1),(0,1),(0,1)]
        for pos,pair,tag,height in elements:
            if pos!='lfe': fields.append((int(pair),1))
            fields.append((tag,4))
        used=sum(w for _,w in fields)
        if used%8: fields.append((0,8-used%8))
        fields += [(len(comment),8)]+[(byte,8) for byte in comment]
        channel=0
        for pos,pair,tag,height in elements:
            fields += [(3 if pos=='lfe' else int(pair),3),(tag,4)]
            if pair: fields += [(0,1)] # separate ICS headers
            for _ in range(2 if pair else 1):
                fields += [(140+4*channel,8),(0,1),(0,2),(0,1),(1,6),(0,1),(1,4),(1,5),(sc[60],sl[60]),(0,1),(0,1),(0,1)]
                index=80 if (frame+channel)%2==0 else 0
                fields += [(codes[index],lengths[index])];channel+=1
        fields += [(7,3)]
        bits=''.join(f'{v:0{w}b}' for v,w in fields);bits+='0'*(-len(bits)%8)
        payload=int(bits,2).to_bytes(len(bits)//8,'big');n=len(payload)+7
        output+=bytes([255,241,76,n>>11,(n>>3)&255,((n&7)<<5)|31,252])+payload
        signals=[]
        for channel in range(channels):
            amplitude=1024.0*2**channel*(1 if (frame+channel)%2==0 else -1)
            block=[sum(amplitude*math.cos(math.pi/1024*(t+0.5+512)*(k+0.5)) for k in range(4))*2/1024/65536*math.sin(math.pi/2048*(t+0.5)) for t in range(2048)]
            signals.append([block[i]+overlaps[channel][i] for i in range(1024)]);overlaps[channel]=block[1024:]
        for i in range(1024):
            for channel in order: pcm+=struct.pack('<f',signals[channel][i])
    stem='aac-height-'+name
    files={}
    for suffix,data in [('.aac',output),('.f32le',pcm)]:
        p=folder/(stem+suffix);p.write_bytes(data);files[suffix]={'file':p.name,'sha256':hashlib.sha256(data).hexdigest()}
    manifest['cases'][name]={'order':order,'mask':mask,'elements':elements,'artifacts':files}
    if name=='mono-top':
        # The PCE comment starts at byte seven of this originally authored block.
        # Verify exact marker before creating CRC/layer malformed derivatives.
        assert output[14]==0xac
        for bad in ['crc','layer']:
            damaged=bytearray(output)
            if bad=='crc': damaged[16]^=1
            else: damaged[15]=0xc0;damaged[16]=crc(damaged[14:16])
            p=folder/('aac-height-invalid-'+bad+'.aac');p.write_bytes(damaged)
            manifest['cases']['invalid-'+bad]={'artifacts':{'.aac':{'file':p.name,'sha256':hashlib.sha256(damaged).hexdigest()}}}
video=b'YUV4MPEG2 W16 H16 F48000:1024 Ip A1:1 C420jpeg\n'
for frame in range(6): video+=b'FRAME\n'+bytes(16+(x*3+y*5+frame*11)%200 for y in range(16) for x in range(16))+bytes([128])*128
p=folder/'aac-height-companion.y4m';p.write_bytes(video)
manifest['video']={'file':p.name,'sha256':hashlib.sha256(video).hexdigest()}
(folder/'aac-height-generated.json').write_text(json.dumps(manifest,indent=2)+'\n')
