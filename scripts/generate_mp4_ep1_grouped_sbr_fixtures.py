"""Own EP1 LD-SBR prefix/data pairs sharing AUs; offline, no codec oracle run."""
import json
from generate_mp4_ep1_fixtures import SOURCE,word
from generate_mp4_ep1_epochs_fixtures import eld_control
from generate_mp4_ep1_active_chain_fixtures import streams
from generate_he_aac_packet_fixtures import box,boxes,packed
OUT=SOURCE/'mp4-ep1-grouped-sbr'
def grouped(source,groups,mode,mutation=None,partitions=None,edits=None):
    original=streams(source);core=len(original)-groups*2;assert core>0
    if partitions is None:
        assert mode!='custom'
        partitions=[[i]for i in range(core)]
    if mode=='batch':partitions.append(list(range(core,len(original))))
    elif mode=='bridge':
        partitions.append([core])
        partitions.extend([[i,i+1]for i in range(core+1,len(original)-1,2)])
        partitions.append([len(original)-1])
    elif mode!='custom':
        for i in range(groups):
            pair=[core+2*i,core+2*i+1]
            partitions.extend([pair]if mode=='all'or i==groups-1 else[[pair[0]],[pair[1]]])
    assert [i for part in partitions for i in part]==list(range(len(original)))
    parts=list(boxes(source));prefix=b''.join(box(t,p)for t,p in parts[:-1]);payload=bytearray();tracks=[]
    for ordinal,indices in enumerate(partitions):
        raw=[];pads=[]
        for frame in range(len(original[0]['raw'])):
            bits=''.join((edits or {}).get((frame,i),''.join(f'{v:08b}'for v in original[i]['raw'][frame])[:len(original[i]['raw'][frame])*8-original[i]['pads'][frame]])for i in indices)
            if frame==1:
                if mutation=='crc' and len(original)-2 in indices:
                    offset=sum(len(original[i]['raw'][frame])*8-original[i]['pads'][frame]for i in indices if i<len(original)-2)
                    bits=bits[:offset]+str(1-int(bits[offset]))+bits[offset+1:]
                if indices[-1]==len(original)-1:
                    if mutation=='truncate':bits=bits[:sum(len(original[i]['raw'][frame])*8-original[i]['pads'][frame]for i in indices[:-1])]
                    elif mutation=='extra':bits+='00000000'
            data=bytearray(packed(bits));pad=-len(bits)%8
            if pad:data[-1]|=(1<<pad)-1
            raw.append(data);pads.append(pad)
        offset=len(prefix)+8+len(payload);payload.extend(b''.join(raw))
        tables={b'stsz':word(0,0,len(raw))+word(*map(len,raw)),b'stco':word(0,1,offset),b'padb':word(0,len(raw))+bytes((pads[i]<<4)|(pads[i+1]if i+1<len(pads)else 0)for i in range(0,len(pads),2))}
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:p=b''.join(rewrite(k,v)for k,v in boxes(p))
            elif t in tables:p=tables[t]
            elif t==b'tkhd':p=p[:12]+word(ordinal+2)+p[16:]
            return box(t,p)
        # Keep each merged AU's first stream clock; all part clocks represent
        # the same source time and duration in their original rational scales.
        body=b''.join(rewrite(t,p)for t,p in boxes(original[indices[0]]['body'])if t!=b'tref')
        if ordinal:body+=box(b'tref',box(b'dpnd',word(ordinal+1)))
        tracks.append(box(b'trak',body))
    moov=[]
    for tag,body in boxes(parts[-1][1]):
        if tag==b'trak':
            md=dict(boxes(dict(boxes(body))[b'mdia']))
            if md[b'hdlr'][8:12]==b'soun':continue
        if tag==b'mvhd':body=body[:-4]+word(len(tracks)+2)
        moov.append(box(tag,body))
    return prefix+box(b'mdat',payload)+box(b'moov',b''.join(moov+tracks)),len(partitions)
def generate():
    files={};cases=[]
    for n in [480,512]:
      for configuration in range(1,8):
       for dual in [False,True]:
        for crc in [False,True]:
            name=f'n{n}-c{configuration}-x{2 if dual else 1}-crc{int(crc)}';control,gold,count,channels=eld_control(n,configuration,crc,dual);groups={1:1,2:1,3:2,4:3,5:3,6:3,7:4}[configuration]
            files[name+'-control.mp4']=control;files[name+'.f32le']=gold
            for mode in ['all']+(['last']if groups>1 else[]):
                movie,width=grouped(control,groups,mode);video=name+'-'+mode+'.mp4';files[video]=movie
                malformed=[]
                for mutation,error in [('truncate','missing AAC ESC instance'),('extra','unconsumed AAC ESC information bits')]+([('crc','low-delay SBR CRC mismatch')]if crc else[]):
                    bad=name+'-'+mode+'-'+mutation+'.mp4';files[bad]=grouped(control,groups,mode,mutation)[0];malformed.append(dict(video=bad,error=error))
                cases.append(dict(name=name+'-'+mode,video=video,control=name+'-control.mp4',golden=name+'.f32le',rate=48000 if dual else 24000,channels=channels,n=n,streams=width,original_streams=count,groups=groups,mode=mode,malformed=malformed))
    assert len(cases)==96
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own AOT39 EP1 LD-SBR instances 9 and 10 packed into one AU per SBR element, or only the final element. 480/512, configurations 1..7, core/double rates and CRC off/on. Independent scalar native PCM. Canonical dpnd, exact bits and rational clocks. No private media, external codec, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
