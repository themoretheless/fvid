#!/usr/bin/env python3
"""Owned AV1 MP4 library/presentation regressions, explicit offline wrapping."""
import hashlib,json
from generate_av1_configuration_samples import D,units
from generate_av1_spatial_container_samples import mp4
from generate_audio_resample_window_fixture import atom,word

def with_edits(data,entries):
    result=b'';at=0
    while at<len(data):
        size=int.from_bytes(data[at:at+4],'big');kind=data[at+4:at+8];body=data[at+8:at+size]
        assert size>=8 and at+size<=len(data)
        if kind==b'moov':body=with_edits(body,entries)
        elif kind==b'trak':
            edits=word(0,len(entries))+b''.join(word(duration)+int(start).to_bytes(4,'big',signed=True)+b'\0\1\0\0' for duration,start in entries)
            body+=atom(b'edts',atom(b'elst',edits))
        result+=atom(kind,body);at+=size
    return result

def main():
    raw=(D/'av1-temporal-operating-points.obu').read_bytes();packets=[];pending=b''
    sequence=next(piece for kind,piece in units(raw) if kind==1)
    for kind,piece in units(raw):
        if kind==2 and pending:packets.append(pending);pending=b''
        pending+=piece
    if pending:packets.append(pending)
    assert len(packets)==8
    base=mp4(packets,sequence,(64,48))
    records=[]
    for name,entries,indices in [('plain',None,list(range(8))),
        ('repeat',[(2,2),(2,0),(2,2)],[2,3,0,1,2,3]),
        ('leading',[(2,-1),(2,4)],[4,5]),('disjoint',[(2,0),(2,6)],[0,1,6,7])]:
        data=base if entries is None else with_edits(base,entries)
        file='av1-library-'+name+'.mp4';(D/file).write_bytes(data)
        records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),indices=indices))
    for name,size,depth in [('odd10',70,10),('lossless12',48,12)]:
        raw=(D.parent/'av1'/(name+'.obu')).read_bytes()
        sequence=next(piece for kind,piece in units(raw) if kind==1)
        packets=[];pending=b''
        for kind,piece in units(raw):
            if kind==2 and pending:packets.append(pending);pending=b''
            pending+=piece
        if pending:packets.append(pending)
        assert len(packets)==2
        data=mp4(packets,sequence,(size,size))
        file='av1-library-'+name+'.mp4';(D/file).write_bytes(data)
        records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),indices=[0,1],
            width=size,height=size,depth=depth,reference='../av1/'+name+'.yuv'))
    odd=json.loads((D/'av1-restoration-odd-generated.json').read_text())['fixtures']
    for depth in [8,10]:
        source=next(case for case in odd if case['depth']==depth and case['quality']==32 and case['orientation']==0)
        raw=(D/source['file']).read_bytes()
        sequence=next(piece for kind,piece in units(raw) if kind==1)
        data=mp4([raw],sequence,tuple(source['size']))
        file=f'av1-library-odd-geometry-depth{depth}.mp4';(D/file).write_bytes(data)
        records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),indices=[0],
            width=source['size'][0],height=source['size'][1],depth=depth,reference=source['reference']))
    chroma=json.loads((D/'av1-chroma-generated.json').read_text())['fixtures']
    for layout,depth in [('422',8),('422',10),('444',8),('444',12)]:
        source=next(case for case in chroma if case['layout']==layout and case['depth']==depth and case['lossless'])
        raw=(D/source['file']).read_bytes();sequence=next(piece for kind,piece in units(raw) if kind==1)
        packets=[];pending=b''
        for kind,piece in units(raw):
            if kind==2 and pending:packets.append(pending);pending=b''
            pending+=piece
        if pending:packets.append(pending)
        assert len(packets)==source['frames']
        data=mp4(packets,sequence,tuple(source['size']))
        file=f'av1-library-chroma{layout}-depth{depth}.mp4';(D/file).write_bytes(data)
        records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),indices=list(range(len(packets))),
            width=source['size'][0],height=source['size'][1],depth=depth,reference=source['reference'],
            subsampling=source['subsampling']))
    (D/'av1-library.json').write_text(json.dumps(dict(cases=records,width=64,height=48,
        reference='av1-temporal-operating-points-op0.yuv',
        provenance='Owned temporal-SVC sequence/packets and saved independent pixel oracle; authored MP4 edit lists. No private media, FFmpeg, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
