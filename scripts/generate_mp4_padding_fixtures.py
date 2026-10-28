"""Own MP4 padb metadata regressions, offline and explicitly generated."""
import json,struct
from pathlib import Path
from generate_he_aac_packet_fixtures import box,boxes
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'mp4-padding'
def with_padding(seed,payload,duplicate=False):
    changed=0
    def rewrite(tag,body):
        nonlocal changed
        if tag in [b'moov',b'trak',b'mdia',b'minf']:
            body=b''.join(rewrite(t,p)for t,p in boxes(body))
        elif tag==b'stbl':
            children=list(boxes(body));stsd=next(p for t,p in children if t==b'stsd')
            if list(boxes(stsd[8:]))[0][0]==b'mp4a':
                changed+=1;body+=box(b'padb',payload)*(2 if duplicate else 1)
        return box(tag,body)
    assert list(boxes(seed))[-1][0]==b'moov'
    result=b''.join(rewrite(t,p)for t,p in boxes(seed));assert changed==1
    return result

def audio_table(seed):
    for t,p in boxes(seed):
        if t==b'moov':
            for t,p in boxes(p):
                if t==b'trak':
                    mdia=next(p for t,p in boxes(p)if t==b'mdia');minf=next(p for t,p in boxes(mdia)if t==b'minf');stbl=next(p for t,p in boxes(minf)if t==b'stbl');children=list(boxes(stbl));stsd=next(p for t,p in children if t==b'stsd')
                    if list(boxes(stsd[8:]))[0][0]==b'mp4a':return children
    raise AssertionError('no AAC track')

def fragment_cases():
    from generate_vp9_chroma import frame,mp4,word
    packet=frame(8,'420',16,16);files={};rows=[]
    def rewrite(data,empty):
        result=b''
        for tag,body in boxes(data):
            if tag in [b'trak',b'mdia',b'minf',b'stbl']:
                body=rewrite(body,empty)
                if tag==b'stbl' and not empty:body+=box(b'padb',word(0,1)+b'\x37')
            elif empty and tag in [b'stts',b'stsc',b'stco',b'stss']:body=word(0,0)
            elif empty and tag==b'stsz':body=word(0,0,0)
            result+=box(tag,body)
        return result
    for name,default,header,first,per,hybrid in [
        ('trex',5,None,None,None,False),('tfhd-zero',5,0,None,None,False),
        ('first',5,3,7,None,False),('rows',5,3,None,[1,0,6],False),
        ('hybrid-odd',5,3,None,[1,0,6],True)]:
        source=dict(boxes(mp4([packet],8,'420',(16,16),[1])))
        init=rewrite(source[b'moov'],not hybrid)
        moov=box(b'moov',init+box(b'mvex',box(b'trex',word(0,1,1,1,len(packet),default<<17))))
        tfhd=box(b'tfhd',word(0x020018|(0x20 if header is not None else 0),1,1,len(packet))+(word(header<<17)if header is not None else b''))
        def make(offset):
            flags=1|(4 if first is not None else 0)|(0x400 if per is not None else 0)
            run=word(flags,3,offset)+(word(first<<17)if first is not None else b'')+(b''.join(word(v<<17)for v in per)if per is not None else b'')
            return box(b'moof',box(b'mfhd',word(0,1))+box(b'traf',tfhd+box(b'tfdt',word(0,int(hybrid)))+box(b'trun',run)))
        probe=make(0);prefix=box(b'ftyp',b'iso5'+word(0)+b'iso5vp09')+(box(b'mdat',source[b'mdat'])if hybrid else b'')+moov
        # Hybrid stco offsets must retain the original prefix length.
        if hybrid:
            prefix=box(b'ftyp',source[b'ftyp'])+box(b'mdat',source[b'mdat'])+moov
        files[name+'.mp4']=prefix+make(len(probe)+8)+box(b'mdat',packet*3)
        padding=per or ([first]+[header if header is not None else default]*2 if first is not None else [header if header is not None else default]*3)
        rows.append(dict(video=name+'.mp4',padding=([3]if hybrid else[])+padding,bytes=len(packet)))
    return files,rows

def generate():
    parent=SOURCE/'aac-ep1';m=json.loads((parent/'manifest.json').read_bytes());case=m['cases'][0];seed=(parent/case['reference_video']).read_bytes()
    table=audio_table(seed);stsz=next(p for t,p in table if t==b'stsz');count=int.from_bytes(stsz[8:12],'big');assert count>2
    fixed=int.from_bytes(stsz[4:8],'big');sizes=[fixed or int.from_bytes(stsz[12+4*i:16+4*i],'big')for i in range(count)]
    padding=[i%8 for i in range(count)];packed=bytes((padding[i]<<4)|(padding[i+1]if i+1<count else 7)for i in range(0,count,2));payload=bytes(4)+struct.pack('>I',count)+packed
    files={'reference.mp4':seed,'padding.mp4':with_padding(seed,payload)};bad=[]
    variants=[('count',payload[:4]+struct.pack('>I',count+1)+payload[8:],'MP4 padding sample count disagrees with size table'),('short',payload[:-1],'invalid MP4 padding table length'),('extra',payload+b'\0','invalid MP4 padding table length'),('version',b'\1'+payload[1:],'unsupported MP4 padding version or flags'),('flags',b'\0\0\0\1'+payload[4:],'unsupported MP4 padding version or flags'),('reserved',payload[:8]+bytes([payload[8]|128])+payload[9:],'nonzero MP4 padding reserved bits'),('duplicate',payload,'duplicate MP4 singleton box')]
    for name,data,error in variants:
        files[name+'.mp4']=with_padding(seed,data,name=='duplicate');bad.append(dict(video=name+'.mp4',error=error))
    extra,fragments=fragment_cases();files.update(extra)
    files['manifest.json']=(json.dumps(dict(sizes=sizes,padding=padding,malformed=bad,fragments=fragments,provenance='Own synthetic ER-LC/AVC source with padb variants. Padding vectors test demux metadata, not valid truncated AAC playback. No private media, foreign codecs, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
