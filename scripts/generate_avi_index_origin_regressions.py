#!/usr/bin/env python3
"""Author equal AVI payloads with either documented idx1 offset origin."""
import hashlib,json,struct
from pathlib import Path
from generate_avi_audio_eof_regressions import generate as owned,chunk,listing
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-index-origin'
def generate():
    original=owned();files={'video.mkv':original['video.mkv']};cases=[]
    for codec,frames,block_frames in [('ima',27,9),('ms',12,4)]:
        base=original[codec+'.avi'];movi=base.index(b'movi');records=[];at=movi+4
        while at<len(base):
            size=struct.unpack_from('<I',base,at+4)[0];records.append(base[at:at+8+size+(size&1)]);at+=8+size+(size&1)
        assert len(records)==4
        for nested in [False,True]:
            payload=b''.join(listing(b'rec ',r) if nested else r for r in records)
            prefix=base[:movi-8];container=prefix+listing(b'movi',payload)
            positions=[];at=movi+4
            for record in records:
                pos=at+12 if nested else at
                positions.append(pos);at+=len(record)+(12 if nested else 0)
            files[f'{codec}-'+('nested' if nested else 'flat')+'-control.avi']=container[:4]+struct.pack('<I',len(container)-8)+container[8:]
            control=f'{codec}-'+('nested' if nested else 'flat')+'-control.avi'
            for origin in ['movi','file']:
                entries=b''.join(struct.pack('<4sIII',r[:4],16,pos-(movi if origin=='movi' else 0),struct.unpack_from('<I',r,4)[0]) for pos,r in zip(positions,records))
                data=container+chunk(b'idx1',entries);data=data[:4]+struct.pack('<I',len(data)-8)+data[8:]
                name=f'{codec}-'+('nested' if nested else 'flat')+'-'+origin+'.avi'
                files[name]=data;cases.append(dict(file=name,control=control,expected=codec+'.f32le',frames=frames,block_frames=block_frames,before_failure=origin=='file',origin=origin,nested=nested))
                if codec=='ima' and not nested:
                    index=data.index(b'idx1')+8
                    for bad in ['mixed-origins','wrong-offset','wrong-length','huge-offset']:
                        if origin!='movi':continue
                        damaged=bytearray(data)
                        field=index+16+8
                        value=positions[1] if bad=='mixed-origins' else positions[1]-movi+1 if bad=='wrong-offset' else 0xffffffff
                        if bad=='wrong-length':field=index+16+12;value=9
                        struct.pack_into('<I',damaged,field,value);files['bad-'+bad+'.avi']=bytes(damaged)
        files[codec+'.f32le']=original[codec+'.f32le']
    files['manifest.json']=(json.dumps(dict(provenance='Eight owned AVI VP9/ADPCM files with identical four records,flat or rec-list grouped,idx1 offsets relative to movi type or file origin. Four independent unindexed controls and literal predictor PCM. Identical VP9 frame decoded in owned Matroska companion;AVI picture playback not claimed. Pure Python, no private media,encoder or network.',reference='https://learn.microsoft.com/en-us/previous-versions/ms779634(v=vs.85)',cases=cases,malformed=['bad-'+s+'.avi' for s in ['mixed-origins','wrong-offset','wrong-length','huge-offset']],sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
