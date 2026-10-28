"""Owned base SL syntax and exact separate-ESC source frames; explicit/offline.
Logical reference movies remain codec projections, not EP1 container routing.
"""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,packed
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'mpeg4-sl'
def config(ts=32,ocr=0,au=16,bitrate=0,priority=0,aseq=0,pseq=0,flags=0xcc,duration=None,initial=(0,0)):
    bits='00000000'+field(flags,8)+field(24000 if ts else 0,32)+field(27000000 if ocr else 0,32)+field(ts,8)+field(ocr,8)+field(au,8)+field(bitrate,8)+field(priority,4)+field(aseq,5)+field(pseq,5)+'11'
    if flags&1:
        scale,a,c=duration;bits+=field(scale,32)+field(a,16)+field(c,16)
    if not flags&4:bits+=field(initial[0],ts)+field(initial[1],ts)
    return packed(bits)
def packet(c,payload,bits,cts=0,dts=None,ocr=None,aseq=0,pseq=0,priority=None,bitrate=None,start=True,end=True,idle=False,padding_only=False,header_padding=False,length_override=None):
    flags=c['flags'];padding=0 if padding_only or idle else(-bits)%8;wire=''
    if flags&128:wire+=field(start,1)
    if flags&64:wire+=field(end,1)
    if c['ocr']:wire+=field(ocr is not None,1)
    if flags&2:wire+=field(idle,1)
    if flags&8:
        wire+=field(padding_only or padding!=0,1)
        if padding_only or padding:wire+=field(padding,3)
    if not idle and not padding_only:
        wire+=field(pseq,c['pseq'])
        if c['priority']:
            wire+=field(priority is not None,1)
            if priority is not None:wire+=field(priority,c['priority'])
        if ocr is not None:wire+=field(ocr,c['ocr'])
        if start:
            if flags&32:wire+='1'
            wire+=field(aseq,c['aseq'])
            if flags&4:wire+=field(dts is not None,1)+field(cts is not None,1)
            if c['bitrate']:wire+=field(bitrate is not None,1)
            if dts is not None:wire+=field(dts,c['ts'])
            if cts is not None and flags&4:wire+=field(cts,c['ts'])
            if c['au']:wire+=field((bits+7)//8 if length_override is None else length_override,c['au'])
            if bitrate is not None:wire+=bitrate
    header_bits=len(wire);wire+=('1'if header_padding else'0')*(-len(wire)%8)
    body=''.join(f'{v:08b}'for v in payload)[:bits]+'0'*padding
    if padding_only:body='10100101'*2
    if idle:body=''
    return packed(wire+body),header_bits

def generate():
    files={};vectors=[];controls=[];bad=[];native=[];blob=bytearray()
    def store(raw):r=dict(offset=len(blob),bytes=len(raw));blob.extend(raw);return r
    for ts in [0,1,31,32,33,63,64]:
      for rate in [0,1,64,65,255]:
        c=dict(ts=ts,ocr=ts,au=32,bitrate=rate,priority=15,aseq=16,pseq=16,flags=0xee)
        setup=config(**c);name=f'w{ts}-r{rate}';files[name+'.slc']=setup
        value=(1<<ts)-1 if ts else None;br=''.join(str((i*7+i//3)%2)for i in range(rate))if rate else None
        wire,header=packet(c,b'\xb6\x80',9,cts=value,dts=None if value is None else value-1,ocr=value,aseq=65535,pseq=65535,priority=32767,bitrate=br,header_padding=True)
        vectors.append(dict(name=name,config=name+'.slc',packet=store(wire),payload='b680',payload_bits=9,header_bytes=(header+7)//8,value=value,bitrate=br))
    for name,flags,start,end,previous,idle,padding_only,error in [
        ('end-start',0x4c,True,True,True,False,False,None),
        ('end-continuation',0x4c,False,True,False,False,False,None),
        ('start-only',0x8c,True,False,None,False,False,None),
        ('implicit-complete',0x0c,True,True,None,False,False,None),
        ('idle',0xce,False,False,None,True,False,None),
        ('padding-only',0xce,False,False,None,False,True,None),
        ('bad-padding-start',0xce,True,False,None,False,True,'padding-only packet carries AU start or OCR'),
        ('unknown-au-length',0xcc,True,True,None,False,False,None),
        ('rap-only',0xdc,True,True,None,False,False,None),
        ('dts-only',0xcc,True,True,None,False,False,None),
        ('bad-au-length',0xcc,True,True,None,False,False,'payload disagrees with AU length'),
    ]:
        c=dict(ts=32,ocr=0,au=16,bitrate=0,priority=0,aseq=3,pseq=3,flags=flags);files[name+'.slc']=config(**c)
        wire,h=packet(c,b'\xda\xa0',13,cts=None if name=='dts-only'else 123,dts=100 if name=='dts-only'else None,aseq=3,pseq=2,start=start,end=end,idle=idle,padding_only=padding_only,length_override=3 if name=='bad-au-length'else 0 if name=='unknown-au-length'else None)
        controls.append(dict(name=name,config=name+'.slc',packet=store(wire),previous_end=previous,start=start,end=end,kind='idle'if idle else'padding'if padding_only else'data',error=error))
    c=dict(ts=64,ocr=64,au=16,bitrate=255,priority=15,aseq=16,pseq=16,flags=0xee);files['absent-fields.slc']=config(**c)
    wire,h=packet(c,b'\xda\xa0',13,cts=None,aseq=3,pseq=2)
    controls.append(dict(name='absent-fields',config='absent-fields.slc',packet=store(wire),previous_end=None,start=True,end=True,kind='data',error=None))
    # Standard predefined defaults and custom out-of-packet start clocks.
    for name,data in [('mp4',b'\x02'),('null',b'\x01'+(123).to_bytes(4,'big')+(456).to_bytes(4,'big')),('initial',config(ts=33,flags=0xc9,duration=(24000,960,1024),initial=((1<<33)-1,19)))]:files[name+'.slc']=data
    for name,data in [('reserved',b'\x03'),('short',b'\0\0'),('width',config(ts=65)),('ocr-width',config(ocr=65)),('au-width',config(au=33)),('au-seq-width',config(aseq=17)),('packet-seq-width',config(pseq=17)),('zero-duration',config(flags=0xcd,duration=(0,1,1))),('zero-clock',config(ts=1)[:2]+bytes(4)+config(ts=1)[6:]),('reserved-bits',config()[:-1]+bytes([config()[-1]&252])),('extra',b'\x02\0')]:files[name+'.slc']=data;bad.append(dict(name=name,config=name+'.slc',error={'reserved':'reserved predefined SL configuration','short':'truncated or oversized bit field','width':'invalid SL configuration field width','ocr-width':'invalid SL configuration field width','au-width':'invalid SL configuration field width','au-seq-width':'invalid SL configuration field width','packet-seq-width':'invalid SL configuration field width','zero-clock':'zero SL timestamp resolution','zero-duration':'zero SL duration timescale','reserved-bits':'invalid SL configuration reserved bits','extra':'extra SL configuration bytes'}[name]))
    # End-to-end packetized ESCs retain actual source bit boundaries.
    parent=SOURCE/'aac-ep1';m=json.loads((parent/'manifest.json').read_bytes());data=(parent/'access-units.bin').read_bytes()
    selected=[c for c in m['cases']if not c['name'].startswith('sbr-')]+[c for c in m['cases']if c['name']in ['sbr-mono-17-15-64-explicit','sbr-multi-19-14-16-64']]
    selected +=[c for c in m['ps_cases']if c['name']in ['sbr-ps-aot17-960-48000-implicit','sbr-ps-aot19-1024-48000-explicit']]
    logical_source=(parent/'logical-packets.bin').read_bytes()
    c=dict(ts=32,ocr=0,au=16,bitrate=0,priority=0,aseq=16,pseq=16,flags=0xcc);files['esc.slc']=config(**c)
    for base in selected:
        rows=[];logical=bytearray();name=base['name'];files[name+'.asc']=bytes.fromhex(base['asc'])
        if not base['ps']:files[name+'.f32le']=(parent/base['golden']).read_bytes()
        files[name+'-logical.mp4']=(parent/base['reference_video']).read_bytes()
        for i,row in enumerate(base['frames']):
            entries=[]
            for u in row['units']:
                raw=data[u['offset']:u['offset']+u['bytes']];wire,_=packet(c,raw,u['bits'],cts=i*base['n'],aseq=i,pseq=i)
                entries.append(dict(stream_id=u['stream_id'],depends_on=u['depends_on'],packet=store(wire),bits=u['bits'],source=raw.hex()))
            wire=logical_source[row['offset']:row['offset']+row['bytes']];rows.append(dict(units=entries,pts=i*base['n'],duration=base['n'],offset=len(logical),bytes=len(wire)));logical.extend(wire)
        files[name+'-logical.bin']=bytes(logical)
        native.append(dict(name=name,ps=base['ps'],signal=base.get('signal'),rate=base['rate'],plain_asc=base['plain_asc'],logical=name+'-logical.bin',config='esc.slc',asc=name+'.asc',golden=name+'.f32le'if not base['ps']else None,reference_video=name+'-logical.mp4',frames=rows))
    files['packets.bin']=bytes(blob);files['manifest.json']=(json.dumps(dict(vectors=vectors,controls=controls,malformed=bad,native=native,provenance='Own SL header fields, exact bit boundaries, nonzero header alignment and zero payload stuffing; owned synthetic ESC data and independent PCM. Logical MP4 reference is not EP1 container playback. No foreign codecs, private media, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
