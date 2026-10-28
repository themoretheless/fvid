"""Own offline SL fragmentation corpus; existing movies are logical projections."""
import json
from generate_sl_fixtures import config,packet,packed,SOURCE
OUT=SOURCE/'mpeg4-sl-assembly'
def generate():
    c=dict(ts=32,ocr=33,au=16,bitrate=0,priority=0,aseq=3,pseq=3,flags=0xec)
    files={'explicit.slc':config(**c)};blob=bytearray();sequences=[]
    def store(raw):
        row=dict(offset=len(blob),bytes=len(raw));blob.extend(raw);return row
    def wire(text,**kw):
        raw,_=packet(c,packed(text),len(text),**kw);return store(raw)
    # Non-byte fragments, modulo wrap, retransmitted OCR and transactional retry.
    first=wire('101',start=True,end=False,pseq=7,aseq=7,ocr=1,length_override=2)
    duplicate=wire('101',start=True,end=False,pseq=7,aseq=7,ocr=(1<<33)-1,length_override=2)
    altered=wire('111',start=True,end=False,pseq=7,aseq=7,ocr=2,length_override=2)
    gap=wire('10011',start=False,end=False,pseq=1)
    middle=wire('10011',start=False,end=False,pseq=0)
    last=wire('11010',start=False,end=True,pseq=1)
    # Same AU seq with RAP is skipped; new sequence wraps to zero.
    repeated=wire('1011001111010',pseq=2,aseq=7)
    nextau=wire('010101010',pseq=3,aseq=0)
    sequences.append(dict(name='explicit-retry-duplicate-wrap',config='explicit.slc',steps=[
        dict(packet=first,units=[]),dict(packet=duplicate,duplicate=True,ocr=(1<<33)-1,units=[]),
        dict(packet=altered,error='nonidentical duplicate SL packet'),
        dict(packet=gap,error='SL packet sequence discontinuity'),dict(packet=middle,units=[]),
        dict(finish=True,error='SL EOF before explicit AU end'),
        dict(packet=last,units=['1011001111010']),dict(packet=repeated,units=[]),
        dict(packet=nextau,units=['010101010']),dict(finish=True,units=[])]))
    # Missing end flags resolve only at the following start or a clean EOF.
    implicit=dict(c,flags=0xac,au=0);files['implicit.slc']=config(**implicit)
    def iw(text,**kw):
        raw,_=packet(implicit,packed(text),len(text),**kw);return store(raw)
    sequences.append(dict(name='implicit-boundaries',config='implicit.slc',steps=[
        dict(packet=iw('1',pseq=0,aseq=0),units=[]),
        dict(packet=iw('01',start=False,pseq=1),units=[]),
        dict(packet=iw('110',pseq=3,aseq=1),error='SL packet sequence discontinuity'),
        dict(packet=iw('110',pseq=2,aseq=1),units=['101']),
        dict(finish=True,units=['110'])]))
    end_only=dict(c,flags=0x6c);files['end-only.slc']=config(**end_only)
    def ew(text,**kw):
        raw,_=packet(end_only,packed(text),len(text),**kw);return store(raw)
    efirst=ew('101',start=True,end=False,pseq=0,aseq=0,length_override=2)
    elast=ew('1001111010',start=False,end=True,pseq=1)
    sequences.append(dict(name='end-only-context-on-duplicate',config='end-only.slc',steps=[
        dict(packet=efirst,units=[]),dict(packet=efirst,duplicate=True,units=[]),
        dict(packet=elast,units=['1011001111010']),dict(packet=elast,duplicate=True,units=[]),
        dict(finish=True,units=[])]))
    nonrap=wire('010',pseq=4,aseq=0,rap=False)
    sequences[0]['steps'].insert(-1,dict(packet=nonrap,units=['010']))
    inactive=dict(c,flags=0xee);files['idle.slc']=config(**inactive)
    def aw(text='',**kw):
        raw,_=packet(inactive,packed(text),len(text),**kw);return store(raw)
    active=aw('101',pseq=0,aseq=0)
    sequences.append(dict(name='idle-sequence-accounting',config='idle.slc',steps=[
        dict(packet=active,units=['101']),dict(packet=aw(idle=True,start=False,end=True),units=[]),
        dict(packet=active,error='SL packet sequence discontinuity'),
        dict(packet=aw('11',pseq=2,aseq=1),units=['11']),
        dict(packet=aw(padding_only=True,start=False,end=True),units=[]),
        dict(packet=aw('01',pseq=4,aseq=2),units=['01']),dict(finish=True,units=[])]))
    native=[];source=SOURCE/'mpeg4-sl';m=json.loads((source/'manifest.json').read_bytes())
    nc=dict(c,ocr=0,aseq=16,pseq=16,flags=0xcc);files['native.slc']=config(**nc)
    files['end-only-native.slc']=config(**dict(nc,flags=0x4c))
    files['clock16.slc']=config(**dict(nc,ts=16))
    for base,mode in [(b,e)for b in m['native']if not b['ps']for e in ['explicit','end-only','wrap','negative-wrap']]:
        end_only=mode=='end-only';anchor=65500 if mode=='wrap'else -37 if mode=='negative-wrap'else 0
        nc['flags']=0x4c if end_only else 0xcc;nc['ts']=16 if mode in ['wrap','negative-wrap']else 32
        counts={};au_counts={};frames=[]
        for frame_index,frame in enumerate(base['frames']):
            units=[]
            for unit in frame['units']:
                sid=unit['stream_id'];au_seq=au_counts.get(sid,0);au_counts[sid]=(au_seq+1)&65535;text=''.join(f'{v:08b}'for v in bytes.fromhex(unit['source']))[:unit['bits']]
                fragments=[];offset=0;part=0
                if not text:
                    sequence=counts.get(sid,65534);counts[sid]=(sequence+1)&65535
                    raw,_=packet(nc,b'',0,start=True,end=True,pseq=sequence,aseq=au_seq,cts=(anchor+frame['pts'])% (1<<nc['ts']),length_override=0)
                    fragments.append(store(raw))
                while offset<len(text):
                    # Every AU begins at an unaligned split; later splits span bytes.
                    width=min([1,7,13,97][part%4],len(text)-offset);last=offset+width==len(text)
                    sequence=counts.get(sid,65534);counts[sid]=(sequence+1)&65535
                    raw,_=packet(nc,packed(text[offset:offset+width]),width,start=offset==0,end=last,pseq=sequence,aseq=au_seq,cts=(anchor+frame['pts'])% (1<<nc['ts']),length_override=(len(text)+7)//8)
                    fragments.append(store(raw));offset+=width;part+=1
                units.append(dict(stream_id=sid,depends_on=unit['depends_on'],bits=len(text),source=unit['source'],fragments=fragments))
            frames.append(dict(pts=frame['pts'],duration=frame['duration'],units=units))
        native.append(dict(name=base['name']+(''if mode=='explicit'else'-'+mode),clock_anchor=anchor,config='end-only-native.slc'if end_only else'clock16.slc'if nc['ts']==16 else'native.slc',asc=base['asc'],golden=base['golden'],reference_video=base['reference_video'],frames=frames))
    files['packets.bin']=bytes(blob)
    files['manifest.json']=(json.dumps(dict(sequences=sequences,native=native,provenance='Own SL packets and synthetic ESC sources; no foreign codecs, private media, processes or network. Logical source movies remain projections, not EP1 container playback.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
