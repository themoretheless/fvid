"""Authored VP9 and tx3g Unicode encodings; generation has no external dependency."""
from pathlib import Path
import struct,json,hashlib
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'tests/fixtures/playback-errors/mp4-subtitle-negative-clock'
OUT=ROOT/'tests/fixtures/playback-errors/mp4-subtitle-unicode'
STANDARD='https://www.etsi.org/deliver/etsi_ts/126200_126299/126245/10.00.00_60/ts_126245v100000p.pdf'
def main():
    source=(BASE/'zero-50.mp4').read_bytes();parts=list(atoms(source));assert [k for k,b in parts]==[b'ftyp',b'mdat',b'moov',b'mdat']
    texts=['ASCII text','<i>Привет</i>\\N🚲','control'];plain=['ASCII text','Привет\n🚲','control']
    style=atom(b'styl',struct.pack('>HHHHBB4B',1,0,1,1,0,12,255,255,255,255))
    def encode(t,encoding):return t.encode('utf-8') if encoding=='utf8' else (b'\xfe\xff'+t.encode('utf-16-be') if encoding=='be' else b'\xff\xfe'+t.encode('utf-16-le'))
    def pack(payload,form,terminate=True):
        # A BOM-only empty string has no character1 to receive a style.
        decor=style[:12]+b'\0\0'+style[14:] if payload in [b'',b'\xfe\xff',b'\xff\xfe'] else style
        if form=='boxed':return atom(b'text',payload+(b'\0\0Ignored font' if terminate else b''))+atom(b'junk',b'ignored')
        if form=='group-zero':return b'\0\0'+struct.pack('>H',len(payload))+payload
        if form=='group-style':return struct.pack('>H',len(decor))+decor+struct.pack('>H',len(payload))+payload
        return struct.pack('>H',len(payload))+payload+(decor if form=='styled' else b'')
    def movie(data,packets,subtitle=False):
        out=b''
        for k,b in atoms(data):
            selected=subtitle
            if k==b'trak':selected=b'sbtl' in b
            if k in [b'trak',b'mdia',b'minf',b'stbl']:b=movie(b,packets,selected)
            elif k==b'stsz' and subtitle:b=word(0,0,3,*map(len,packets))
            out+=atom(k,b)
        return out
    files={f'frame-{i}.yuv16':(BASE/f'frame-{i}.yuv16').read_bytes() for i in range(3)};cases=[];refusals=[]
    def add(name,payloads,form,expected,refusal=False):
        packets=[pack(p,form,not (refusal and i==0)) for i,p in enumerate(payloads)]
        moov=movie(parts[2][1],packets);assert len(moov)==len(parts[2][1])
        files[name]=b''.join(atom(k,b) for k,b in parts[:2])+atom(b'moov',moov)+atom(b'mdat',b''.join(packets))
        for i,p in enumerate(packets):files[f'{name}-{i}.packet']=p
        c=dict(file=name,expected=expected,form=form)
        (refusals if refusal else cases).append(c)
    for name,encoding,form in [('utf8-standard','utf8','standard'),('utf16be-standard','be','standard'),('utf16le-standard','le','standard'),('utf16be-styled','be','styled'),('utf16be-group-zero','be','group-zero'),('utf16be-group-style','be','group-style'),('utf16be-boxed','be','boxed')]:
        add(name+'.mp4',[encode(t,encoding) for t in texts],form,plain)
    add('utf16be-empty.mp4',[b'\xfe\xff',encode('healthy','utf8'),encode('control','utf8')],'standard',['healthy','control'])
    add('utf8-control-prefix.mp4',[b'\0\0\0\x08test',encode('healthy','utf8'),encode('control','utf8')],'standard',['\0\0\0\x08test','healthy','control'])
    for form in ['styled','group-zero','group-style','boxed']:
        add(f'utf16be-empty-{form}.mp4',[b'\xfe\xff',encode('healthy','utf8'),encode('control','utf8')],form,['healthy','control'])
    for name,bad in [('odd',b'\xfe\xff\0A\0'),('high-surrogate',b'\xfe\xff\xd8\0'),('low-surrogate',b'\xfe\xff\xdc\0')]:
        for form in ['standard','styled','group-zero','group-style','boxed']:
            add(f'utf16be-invalid-{name}-{form}.mp4',[bad,encode('healthy','utf8'),encode('control','utf8')],form,['healthy','control'],True)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Authored16x16 colored VP9, Unicode strings, UTF-8/UTF-16 BOM and complete tx3g defaults/font table. Standard samples plus supported legacy grouped/boxed wrappers. No private media/parameters, FFmpeg or network.',standard=STANDARD,sections=['5.1','5.16','5.17'],normative_basis='UTF-16 with big-endian BOM is required; reversed byte order is optional. String lengths count bytes; the font table declares the default font.',cases=cases,malformed_cue_refusals=refusals,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2,ensure_ascii=False)+'\n')
if __name__=='__main__':main()
