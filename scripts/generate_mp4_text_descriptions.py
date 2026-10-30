"""Authored text sample-description selection, including real text fragments."""
from pathlib import Path
import hashlib,json,struct
from generate_mp4_seek_io_regressions import generate
from generate_mp4_subtitle_negative_clock import text_entry
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-text-descriptions'
def main():
    own=generate();parts=list(atoms(own['three-colors.mp4']));prefix=b''.join(atom(k,b) for k,b in parts[:2]);base=parts[2][1];vtrack=next(b for k,b in atoms(base) if k==b'trak');texts=['First caption','Middle caption','Last caption'];tx=[struct.pack('>H',len(t))+t.encode() for t in texts];wv=[atom(b'vttc',atom(b'payl',t.encode())) for t in texts]
    def unknown(reference=1):return atom(b'wvtt',bytes(6)+struct.pack('>H',reference)+atom(b'vttC',b'WEBVTT\n\n'))
    def subtitle(data,offset,handler,entries,indices,payloads,fragmented):
        out=b''
        for k,b in atoms(data):
            if k in [b'mdia',b'minf']:b=subtitle(b,offset,handler,entries,indices,payloads,fragmented)
            elif k==b'tkhd':b=word(0)+b[4:12]+word(2)+b[16:20]+word(9)+b[24:]
            elif k==b'hdlr':b=word(0,0)+handler.encode()+bytes(12)+b'Authored text\0'
            elif k==b'vmhd':k=b'nmhd';b=word(0)
            elif k==b'stbl':
                b=atom(b'stsd',word(0,len(entries))+b''.join(entries))
                if fragmented:b+=atom(b'stts',word(0,0))+atom(b'stsz',word(0,0,0))+atom(b'stsc',word(0,0))+atom(b'stco',word(0,0))
                else:
                    positions=[offset,offset+len(payloads[0]),offset+len(payloads[0])+len(payloads[1])]
                    b+=atom(b'stts',word(0,1,3,1))+atom(b'stsz',word(0,0,3,*map(len,payloads)))+atom(b'stsc',word(0,3)+b''.join(word(i+1,1,d) for i,d in enumerate(indices)))+atom(b'stco',word(0,3,*positions))
            out+=atom(k,b)
        return out
    def alternate(data):
        out=b''
        for k,b in atoms(data):
            if k==b'tkhd':b=word(0)+b[4:12]+word(3)+b[16:]
            out+=atom(k,b)
        return out
    def clip(handler,variant,fragmented=False,text_first=False,extra_video=False,bad=False):
        entries=[text_entry()] if variant=='plain' else ([unknown(2 if bad else 1),text_entry()] if variant=='unused-first' else [text_entry(),unknown(2 if bad else 1)])
        indices={'plain':[1]*3,'unused-first':[2]*3,'unused-last':[1]*3,'used-unknown':[2]*3,'mixed-used':[1,2,1]}[variant]
        codecs=[b[4:8].decode() for b in entries];payloads=[tx[i] if codecs[d-1]=='tx3g' else wv[i] for i,d in enumerate(indices)]
        def movie(offset):
            out=b''
            for k,b in atoms(base):
                if k==b'mvhd':b=b[:16]+word(9)+b[20:-4]+word(4 if extra_video else 3)
                if k!=b'trak':out+=atom(k,b)
                else:
                    text=atom(b'trak',subtitle(vtrack,offset,handler,entries,indices,payloads,fragmented));video=atom(k,b)
                    out+=text+video if text_first else video+text
                    if extra_video:out+=atom(b'trak',alternate(vtrack))
            if fragmented:out+=atom(b'mvex',atom(b'trex',word(0,2,1,1,0,0)))
            return out
        moov=movie(0);offset=len(prefix)+len(atom(b'moov',moov))+8;moov=movie(offset)
        result=prefix+atom(b'moov',moov)
        if not fragmented:result+=atom(b'mdat',b''.join(payloads))
        else:
            for i,(d,p) in enumerate(zip(indices,payloads)):
                def fragment(offset):
                    traf=atom(b'tfhd',word(0x02000a,2,d,1))+atom(b'tfdt',word(0,i))+atom(b'trun',word(0x201,1)+struct.pack('>i',offset)+word(len(p)))
                    return atom(b'moof',atom(b'mfhd',word(0,i+1))+atom(b'traf',traf))
                moof=fragment(0);result+=fragment(len(moof)+8)+atom(b'mdat',p)
        return result,codecs,indices,payloads
    files={f'frame-{i}.yuv16':own[f'frame-{i}.yuv16'] for i in range(3)};cases=[]
    for i,p in enumerate(tx):files[f'tx3g-{i}.packet']=p
    for i,p in enumerate(wv):files[f'wvtt-{i}.packet']=p
    for handler in ['sbtl','text','subt']:
        for fragmented in [False,True]:
            for variant in ['plain','unused-first','unused-last','used-unknown','mixed-used']:
                for text_first in ([False,True] if variant in ['unused-first','used-unknown'] else [False]):
                    name=f'{handler}-{"fragmented" if fragmented else "classic"}-{variant}-{"text-first" if text_first else "video-first"}.mp4';data,codecs,indices,_=clip(handler,variant,fragmented,text_first);files[name]=data;cases.append(dict(file=name,handler=handler,mode='fragmented' if fragmented else 'classic',variant=variant,codecs=codecs,indices=indices,available=variant in ['plain','unused-first','unused-last'],texts=texts))
    n='header-alignment-after-refusal.mp4';data,codecs,indices,_=clip('sbtl','used-unknown',True,True,True);files[n]=data;cases.append(dict(file=n,handler='sbtl',mode='fragmented',variant='used-unknown',codecs=codecs,indices=indices,available=False,texts=texts,extra_video=True))
    data,_,_,_=clip('sbtl','unused-first',bad=True);files['bad-unused-reference.mp4']=data
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Pure authored16x16 three-color VP9 plus invented tx3g/WebVTT text. stsd entries are selected by explicit classic stsc or real fragment tfhd descriptions; no private media/parameters, FFmpeg or network. Text header is disabled/duration9, video header enabled/duration3; an extra disabled video checks parallel-header alignment after refusal.',reference='https://developer.apple.com/documentation/quicktime-file-format/sample_description_atom',cases=cases,refused=['bad-unused-reference.mp4'],sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
