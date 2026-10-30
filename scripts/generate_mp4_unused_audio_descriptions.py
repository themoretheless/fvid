"""Own PCM transitions with opaque unselected audio SampleEntry records."""
from pathlib import Path
import json,hashlib,struct
from generate_mp4_audio_reconfiguration import generate
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-unused-audio-descriptions'
def main():
    files={};cases=[]
    for profile,shapes in [('changing',None),('constant',[(b'sowt',48000,2,16)]*3)]:
        own=generate(pcm_shapes=shapes);meta=json.loads(own['manifest.json']);frames=meta['frames']
        for f in frames:
            old=f['oracle'];f['oracle']=profile+'-'+old;files[f['oracle']]=own[old]
        for container,src in [('regular','pcm.mp4'),('fragmented','pcm-fragmented.mp4')]+([('compact','pcm.mp4')] if profile=='constant' else []):
            source=own[src];files[f'{profile}-{container}-control.mp4']=source
            for insert in range(4):
                def transform(data,audio=False,referenced=False,reference=1):
                    out=b''
                    for k,b in atoms(data):
                        selected=audio
                        if k==b'trak':
                            mdia=dict(atoms(dict(atoms(b))[b'mdia']));selected=mdia[b'hdlr'][8:12]==b'soun'
                        elif k==b'traf':selected=struct.unpack_from('>I',dict(atoms(b))[b'tfhd'],4)[0]==2
                        if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl',b'mvex',b'moof',b'traf']:b=transform(b,selected,referenced,reference)
                        elif k==b'stsd' and audio:
                            entries=[atom(x,y) for x,y in atoms(b[8:])];assert len(entries)==3
                            opaque=bytes(6)+struct.pack('>H',reference)+b'owned opaque audio'
                            entries.insert(insert,atom(b'zzzz',opaque));b=word(0,4)+b''.join(entries)
                        elif k==b'stsz' and audio and container=='compact':
                            sizes=struct.unpack('>6I',b[12:]);assert len(set(sizes))==1
                            b=word(0,sizes[0],6)
                        elif k==b'stsc' and audio:
                            b=bytearray(b)
                            for i in range(struct.unpack_from('>I',b,4)[0]):
                                at=16+i*12;ident=struct.unpack_from('>I',b,at)[0];struct.pack_into('>I',b,at,insert+1 if referenced and i==0 else ident+(ident>insert))
                            b=bytes(b)
                        elif k==b'trex' and struct.unpack_from('>I',b,4)[0]==2:
                            ident=struct.unpack_from('>I',b,8)[0];b=b[:8]+word(ident+(ident>insert))+b[12:]
                        elif k==b'tfhd' and audio:
                            flags=struct.unpack_from('>I',b)[0];assert flags&2
                            at=8+(8 if flags&1 else 0);ident=struct.unpack_from('>I',b,at)[0];b=b[:at]+word(ident+(ident>insert))+b[at+4:]
                        out+=atom(k,b)
                    return out
                n=f'{profile}-{container}-insert-{insert}.mp4';files[n]=transform(source);cases.append(dict(file=n,control=f'{profile}-{container}-control.mp4',frames=frames,insert=insert))
                if profile=='changing' and container=='regular' and insert==0:
                    files['referenced-unknown.mp4']=transform(source,referenced=True)
                    files['unreferenced-external.mp4']=transform(source,reference=2)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Owned VP9 companion and six authored nonzero PCM spans (changing coding/rate/channels/precision or equivalent constant configurations). Unknown zzzz is an opaque own SampleEntry with valid common data reference and no private parameters. No FFmpeg/network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
