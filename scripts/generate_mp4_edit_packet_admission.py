"""Own finite PCM edit admission witnesses,with valid larger hidden packet."""
from pathlib import Path
import json,hashlib,struct
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-edit-packet-admission'
def generate():
    base=OUT.parent/'mp4-concat-edit-work';raw=(base/'video-control.mp4').read_bytes();pcm=(base/'video-control.mp4.f32le').read_bytes();files={}
    def transform(data,video,edited):
        out=b'';fragment=-1
        def movie(data,audio=False):
            out=b''
            for k,b in atoms(data):
                if k==b'trak' and b'vide' in b and not video:continue
                a=audio or (k==b'trak' and b'soun' in b)
                if k==b'edts' and a and not edited:continue
                if k in [b'moov',b'trak',b'edts']:b=movie(b,a)
                elif k==b'tkhd' and a and not edited:b=b[:20]+word(480)+b[24:]
                out+=atom(k,b)
            return out
        def traf(data):
            out=b''
            for k,b in atoms(data):
                if k==b'traf':b=traf(b)
                elif k==b'tfhd':b=b[:-4]+word(640)
                out+=atom(k,b)
            return out
        for k,b in atoms(data):
            if k==b'moov':b=movie(b)
            elif k==b'moof':
                fragment+=1
                if fragment==2:b=traf(b)
            elif k==b'mdat' and fragment==2:b=b+b
            out+=atom(k,b)
        return out
    for video in [False,True]:
        suffix='mp4' if video else 'm4a'
        for edited in [True,False]:
            name=('early'if edited else 'full')+'.'+suffix;files[name]=transform(raw,video,edited)
    files['early.f32le']=pcm
    # All three authored original packet windows remain valid in full controls.
    files['full.f32le']=(OUT.parent/'mp4-open-ended-edit/pcm-oracle.f32le').read_bytes()
    cases=[]
    reference=OUT.parent/'mp4-open-ended-edit'
    for codec in ['pcm','alac']:
        template=(reference/f'{codec}-v0-clock8000-trim0-known.mp4').read_bytes()
        oracle=(reference/f'{codec}-oracle.f32le').read_bytes()
        original_packets=[b for k,b in atoms(template) if k==b'mdat'][-3:]
        if codec=='pcm':last=original_packets[-1]*2
        else:
            values=struct.unpack('<160f',oracle[320*4:480*4]);values=[round(v*32768)for v in values]*2
            bits='000'+'0000'+'0'*12+'1'+'00'+'1'+f'{len(values):032b}'+''.join(f'{v&65535:016b}'for v in values)+'111';bits+='0'*(-len(bits)%8);last=int(bits,2).to_bytes(len(bits)//8,'big')
        def build(clock,version,rows):
            duration=sum(d for d,t in rows)if rows is not None else (480*clock+7999)//8000
            def movie(data,audio=False):
                out=b''
                for k,b in atoms(data):
                    a=audio or(k==b'trak' and b'soun' in b)
                    if k==b'edts' and a and rows is None:continue
                    if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl',b'edts']:b=movie(b,a)
                    elif k==b'mvhd':b=b[:12]+word(clock,max(duration,(480*clock+7999)//8000))+b[20:]
                    elif k==b'tkhd':b=b[:20]+word(duration if a else (int.from_bytes(b[20:24],'big')*clock+7999)//8000)+b[24:]
                    elif k==b'elst' and a:b=word(version<<24,len(rows))+b''.join(struct.pack('>IiI',d,t,65536)if version==0 else struct.pack('>QqI',d,t,65536)for d,t in rows)
                    elif k==b'stsd' and a and codec=='alac':
                        # Nested ALAC atom follows the 28-byte sample-entry header.
                        cookie=8+8+28+8+4
                        assert int.from_bytes(b[cookie:cookie+4],'big')==160
                        b=b[:cookie]+word(320)+b[cookie+4:]
                    out+=atom(k,b)
                return out
            def fragment(data):
                out=b''
                for k,b in atoms(data):
                    if k==b'traf':b=fragment(b)
                    elif k==b'tfhd':b=b[:-4]+word(len(last))
                    out+=atom(k,b)
                return out
            out=b'';i=-1
            for k,b in atoms(template):
                if k==b'moov':b=movie(b)
                elif k==b'moof':
                    i+=1
                    if i==2:b=fragment(b)
                elif k==b'mdat' and i==2:b=last
                out+=atom(k,b)
            return out
        files[f'{codec}-full-control.mp4']=build(8000,0,None)
        files[f'{codec}-full-control.f32le']=oracle
        for clock in [8000,1000,1001]:
            unit=160 if clock==8000 else 20
            for version in [0,1]:
                for label,rows in [('early',[(unit,0)]),('preroll',[(unit,160)]),('repeat',[(unit,0),(unit,0)]),('gap',[(unit,-1),(unit,0)]),('silence',[(unit,-1)])]:
                    name=f'{codec}-clock{clock}-v{version}-{label}.mp4';files[name]=build(clock,version,rows)
                    expected=b'';ticks=0;frames=0;work=0
                    for d,start in rows:
                        ticks+=d;end=(ticks*8000+clock-1)//clock;length=end-frames;frames=end
                        if start<0:expected+=bytes(length*4)
                        else:expected+=oracle[start*4:(start+length)*4];work+=start//160+1
                    files[name+'.f32le']=expected;cases.append(dict(file=name,oracle=name+'.f32le',frames=frames,work=work,cap=400))
    files['manifest.json']=(json.dumps(dict(provenance='Authored neutral VP9 and analytic PCM. A valid 640-byte final packet is beyond the finite first-packet edit; full controls retain all packet windows.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
