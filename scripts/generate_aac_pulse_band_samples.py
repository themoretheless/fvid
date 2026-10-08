#!/usr/bin/env python3
"""Original AAC-LC pulse/band interactions; explicit offline generation."""
from pathlib import Path
import hashlib,json,math,re,struct
root=Path(__file__).resolve().parents[1];folder=root/'tests/fixtures/playback-errors'
tables=(root/'crates/fvid-media/src/owned_aac/aac_huffman_tables.rs').read_text()
def table(name):
    body=re.search(r'const '+name+r':[^=]+ = \[(.*?)\];',tables,re.S)[1]
    return [int(n,0) for n in re.findall(r'0x[0-9a-fA-F]+|\d+',body)]
codes=table('SPECTRUM_CODEBOOK1_CODES');lens=table('SPECTRUM_CODEBOOK1_LENS');sc=table('SCF_CODEBOOK_CODES');sl=table('SCF_CODEBOOK_LENS')
# kind, start band, relative offsets/amplitudes; source bands begin 0,4,8.
cases={
 'outside':('spectral',1,[(0,15)]),
 'outside-four':('spectral',1,[(0,1),(0,2),(0,3),(0,4)]),
 'outside-far':('spectral',48,[(31,15)]),
 'cross-boundary':('spectral',0,[(3,2),(1,4)]),
 'repeated-spectral':('spectral',0,[(0,15)]*4),
 'zero-spectral':('zero-spectral',0,[(0,15)]),
 'zero-band':('zero',1,[(0,15)]),
 'noise-band':('noise',1,[(0,15)]),
 'intensity14':('intensity14',1,[(0,15)]),
 'intensity15':('intensity15',1,[(0,15)]),
 'invalid-start':('spectral',49,[(0,15)]),
 'invalid-offset':('spectral',48,[(31,15)]*4),
}
manifest={'cases':{}}
for name,(kind,start,pulses) in cases.items():
    stereo=kind.startswith('intensity');channels=2 if stereo else 1
    for enabled in [True,False]:
        output=bytearray();reference=bytearray();overlap=[0.0]*1024
        for frame in range(6):
            books=[1] if kind in ['spectral','zero-spectral'] else [1,{'zero':0,'noise':13,'intensity14':14,'intensity15':15}[kind]]
            def channel(books,with_pulse):
                fields=[(140,8)]
                if not stereo: fields += [(0,1),(0,2),(0,1),(len(books),6),(0,1)]
                for book in books: fields += [(book,4),(1,5)]
                for book in books:
                    if book==13: fields += [(256,9)]
                    elif book: fields += [(sc[60],sl[60])]
                fields += [(int(with_pulse),1)]
                if with_pulse: fields += [(len(pulses)-1,2),(start,6)]+[(v,w) for offset,amp in pulses for v,w in [(offset,5),(amp,4)]]
                fields += [(0,1),(0,1)] # TNS and gain control absent
                for book in books:
                    if book==1:
                        index=40 if kind=='zero-spectral' else 80 if frame%2==0 else 0
                        fields += [(codes[index],lens[index])]
                return fields
            fields=[(int(stereo),3),(0,4)]
            if stereo:
                fields += [(1,1),(0,1),(0,2),(0,1),(2,6),(0,1),(0,2)] # common ICS, ms_mask_present=0
                fields += channel([1,1],False)+channel(books,enabled)
            else:fields += channel(books,enabled)
            fields += [(7,3)]
            bits=''.join(f'{v:0{w}b}' for v,w in fields);bits+='0'*(-len(bits)%8)
            payload=int(bits,2).to_bytes(len(bits)//8,'big');n=len(payload)+7
            output += bytes([255,241,76,(channels<<6)|(n>>11),(n>>3)&255,((n&7)<<5)|31,252])+payload
            if kind not in ['noise','intensity14','intensity15'] and not name.startswith('invalid'):
                values=[0 if kind=='zero-spectral' else (1 if frame%2==0 else -1)]*4
                if enabled and start==0:
                    position=0
                    for offset,amp in pulses:
                        position+=offset
                        if position<4:values[position]+=amp if values[position]>0 else -amp
                spectrum=[math.copysign(abs(q)**(4/3)*1024,q) if q else 0.0 for q in values]
                block=[sum(v*math.cos(math.pi/1024*(t+0.5+512)*(k+0.5)) for k,v in enumerate(spectrum))*2/1024/65536*math.sin(math.pi/2048*(t+0.5)) for t in range(2048)]
                reference += b''.join(struct.pack('<f',block[i]+overlap[i]) for i in range(1024));overlap=block[1024:]
        stem=f'aac-pulse-band-{name}'+('' if enabled else '-baseline');artifacts={}
        for ext,data in [('.aac',output)]+([('.f32le',reference)] if reference else []):
            p=folder/(stem+ext);p.write_bytes(data);artifacts[ext]={'file':p.name,'sha256':hashlib.sha256(data).hexdigest()}
        manifest['cases'][name+('' if enabled else '-baseline')]={'kind':kind,'pulse_start':start,'pulses':pulses if enabled else [],'artifacts':artifacts}
video=b'YUV4MPEG2 W16 H16 F48000:1024 Ip A1:1 C420jpeg\n'
for frame in range(6):video+=b'FRAME\n'+bytes(32+(x*5+y*7+frame*9)%160 for y in range(16) for x in range(16))+bytes([128])*128
p=folder/'aac-pulse-band-companion.y4m';p.write_bytes(video);manifest['video']={'file':p.name,'sha256':hashlib.sha256(video).hexdigest()}
(folder/'aac-pulse-band-generated.json').write_text(json.dumps(manifest,indent=2)+'\n')
