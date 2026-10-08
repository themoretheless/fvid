#!/usr/bin/env python3
"""Original absolute PS parameter choices, encoded as frequency/time deltas.

Expected native-band indices come from chosen targets, not decoder output.
Decimal/Machin gives independent high-precision phase dequantization oracles.
No foreign codec, FFmpeg, libav, private media or network. Explicit offline
generation only; ordinary tests consume saved assets.
"""
from decimal import Decimal as D, localcontext
import hashlib
import json
from generate_aac_ps_fixtures import BOOKS, sized, sbr
from generate_he_aac_packet_fixtures import DEST, packed, field, packet, asc, video_fixture

COARSE = [-25,-18,-14,-10,-7,-4,-2,0,2,4,7,10,14,18,25]
FINE = [-50,-45,-40,-35,-30,-25,-22,-19,-16,-13,-10,-8,-6,-4,-2,0,2,4,6,8,10,13,16,19,22,25,30,35,40,45,50]
ICC = ['1','.937','.84118','.60092','.36764','0','-.589','-1']

def zero(iid_mode=0,icc_mode=0):
    return dict(iid_mode=iid_mode,icc_mode=icc_mode,phase_mode=iid_mode,
                iid_enabled=False,icc_enabled=False,phase_enabled=False,
                iid=[0]*[10,20,34][iid_mode%3],icc=[0]*[10,20,34][icc_mode%3],
                ipd=[0]*[5,11,17][iid_mode%3],opd=[0]*[5,11,17][iid_mode%3])

def targets(iid_mode,icc_mode,step,iid=True,icc=True,phase=True):
    values=zero(iid_mode,icc_mode)
    values.update(iid_enabled=iid,icc_enabled=icc,phase_enabled=phase)
    limit=15 if iid_mode>=3 else 7
    if iid:values['iid']=[(3*b+5*step)%(2*limit+1)-limit for b in range(len(values['iid']))]
    if icc:values['icc']=[(5*b+3*step)%8 for b in range(len(values['icc']))]
    if phase:
        values['ipd']=[(3*b+5*step)%8 for b in range(len(values['ipd']))]
        values['opd']=[(5*b+7*step)%8 for b in range(len(values['opd']))]
    return values

def retain(previous,ei,ec,ep):
    result=dict(previous)
    for key,enabled in [('iid',ei),('icc',ec),('ipd',ep),('opd',ep)]:
        if not enabled:result[key]=[0]*len(previous[key])
    if not ei:result['iid_enabled']=False
    if not ec:result['icc_enabled']=False
    if not ep:result['phase_enabled']=False
    return result

def encode_row(label,mode,target,previous,temporal):
    if temporal:
        references=[previous[b] if b<len(previous) else 0 for b in range(len(target))]
    else:
        references=[0]+target[:-1]
    delta=[v-ref for v,ref in zip(target,references)]
    if label in ['ipd','opd']:delta=[v%8 for v in delta]
    family='IidFine' if label=='iid' and mode>=3 else 'IidCoarse' if label=='iid' else label.title()
    book=BOOKS[family+('Time' if temporal else 'Frequency')]
    return field(temporal,1)+''.join(book[v] for v in delta)

def encode(header_present,iid_mode,icc_mode,enable_iid,enable_icc,phase,rows,
           previous,slots,variable=False,first_time=False,extension=True,explicit_borders=None):
    count=len(rows)
    text=field(header_present,1)
    if header_present:
        text+=field(enable_iid,1)+(field(iid_mode,3) if enable_iid else '')
        text+=field(enable_icc,1)+(field(icc_mode,3) if enable_icc else '')+field(extension,1)
    counts=[1,2,3,4] if variable else [0,1,2,4]
    text+=field(variable,1)+field(counts.index(count),2)
    borders=[(e+1)*slots//(count+1) for e in range(count)] if variable else [(e+1)*slots//count-1 for e in range(count)]
    if explicit_borders is not None:
        assert variable and len(explicit_borders)==count
        assert all(0<=v<slots for v in explicit_borders)
        assert all(a<b for a,b in zip(explicit_borders,explicit_borders[1:]))
        borders=list(explicit_borders)
    if variable:text+=''.join(field(v,5) for v in borders)
    def time(e,label):
        # Every first row is explicitly independently coded or temporal.
        # Within a frame, tools deliberately alternate different directions.
        return first_time if e==0 else ((e+(label in ['icc','opd']))%2==1)
    for label,mode,enabled in [('iid',iid_mode,enable_iid),('icc',icc_mode,enable_icc)]:
        if enabled:
            for e,row in enumerate(rows):
                source=previous if e==0 else rows[e-1]
                text+=encode_row(label,mode,row[label],source[label],time(e,label))
    if extension:
        inner='00'+field(phase,1)
        if phase:
            assert enable_iid
            for e,row in enumerate(rows):
                source=previous if e==0 else rows[e-1]
                for label in ['ipd','opd']:
                    inner+=encode_row(label,iid_mode,row[label],source[label],time(e,label))
        text+=sized(inner+'0')
    return text,borders

def atan(x):
    total=D(0);power=x;sign=1;denominator=1
    while True:
        term=power/denominator
        after=total+sign*term
        if after==total:return total
        total=after;power*=x*x;sign=-sign;denominator+=2

def main():
    binary=bytearray();sequences=[]
    def store(bits,borders,rows,retained,initialized,header_present,header,phase,iid_mode,icc_mode):
        start=(len(binary)+3)%8
        raw=packed('1'*start+bits+'10100101')
        frame=dict(offset=len(binary),bytes=len(raw),start=start,bits=len(bits),
                   expected=dict(header_present=header_present,header=header,phase_enabled=phase,iid_mode=iid_mode,icc_mode=icc_mode,
                                 borders=borders,envelopes=rows,retained=retained,initialized=initialized))
        binary.extend(raw)
        return frame
    for slots in [24,30,32]:
        for mode in range(6):
            old_icc=(mode+2)%6;new_iid=(mode+3)%6;new_icc=(old_icc+3)%6
            previous=zero();frames=[]
            controls=[
                (True,mode,old_icc,True,True,True,2,False,False),
                (False,mode,old_icc,True,True,True,3,True,True),
                (False,mode,old_icc,True,True,True,0,False,False),
                (True,new_iid,new_icc,True,True,True,0,False,False),
                (True,new_iid,new_icc,False,False,False,0,False,False),
                (True,new_iid,new_icc,True,True,True,0,False,False),
                (False,new_iid,new_icc,True,True,True,4,False,False),
                (True,new_iid,new_icc,False,False,False,1,False,False),
                (True,new_iid,new_icc,True,True,True,2,False,True),
                (False,new_iid,new_icc,True,True,False,1,True,True),
                (False,new_iid,new_icc,True,True,True,1,False,True),
            ]
            for step,(hdr,im,cm,ei,ec,ep,count,var,dt) in enumerate(controls):
                rows=[targets(im,cm,step*7+e,ei,ec,ep) for e in range(count)]
                bits,borders=encode(hdr,im,cm,ei,ec,ep,rows,previous,slots,var,dt)
                retained=rows[-1] if rows else retain(previous,ei,ec,ep)
                header=dict(iid_mode=im if ei else None,icc_mode=cm if ec else None,extension=True)
                frames.append(store(bits,borders,rows,retained,True,hdr,header,ep,im,cm))
                previous=retained
            sequences.append(dict(name=f'varying-modes-{mode}-slots-{slots}',slots=slots,frames=frames))
    # Startup: absent header, then a mode-0 header with time-coded first rows,
    # then an independent header. Only the final packet starts stereo DSP.
    previous=zero();frames=[]
    for step,(hdr,ei,ep,dt) in enumerate([(False,False,False,False),(True,True,True,True),(True,True,True,False)]):
        rows=[targets(0,0,step,ei,ei,ep)]
        bits,borders=encode(hdr,0,0,ei,ei,ep,rows,previous,32,first_time=dt,extension=hdr)
        frames.append(store(bits,borders,rows,rows[-1],step==2,hdr,
            dict(iid_mode=0 if ei else None,icc_mode=0 if ei else None,extension=hdr),ep,0,0))
        previous=rows[-1]
    sequences.append(dict(name='startup-needs-independent-header',slots=32,frames=frames))
    # A no-envelope header changes resolution, while the retained physical
    # parameters stay in the old native geometry. Later time rows address raw
    # indices by band, with index zero for previously nonexistent bands.
    previous=zero();frames=[]
    for step,(hdr,mode,count,dt) in enumerate([(True,0,1,False),(True,2,0,False),
            (False,2,1,True),(True,1,0,False),(False,1,1,True)]):
        rows=[targets(mode,mode,step)] if count else []
        bits,borders=encode(hdr,mode,mode,True,True,True,rows,previous,32,first_time=dt)
        retained=rows[-1] if rows else previous
        frames.append(store(bits,borders,rows,retained,True,hdr,
            dict(iid_mode=mode,icc_mode=mode,extension=True),True,mode,mode))
        previous=retained
    sequences.append(dict(name='no-envelope-resolution-change',slots=32,frames=frames))
    # Existing PS video reproduces pending synthesis. This additional video
    # carries varying absolute IID/ICC/IPD/OPD targets and phase wrapping.
    blob=bytearray();packet_frames=[];payloads=[];video_targets=[];previous=zero()
    for stage in range(3):
        rows=[targets(1,1,stage*3+e) for e in range(2)]
        bits,_=encode(stage==0,1,1,True,True,True,rows,previous,32,first_time=stage>0)
        raw=sbr(bits,stage);payloads.append(raw.hex());data=packet(raw)
        packet_frames.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
        video_targets.append(rows);previous=rows[-1]
    malformed=[];bad_blob=bytearray()
    for kind,im in [('iid-coarse',1),('iid-fine',4),('icc',1)]:
        target=targets(im,1,0,phase=False)
        target['iid']=[0]*20;target['icc']=[0]*20
        if kind=='iid-coarse':target['iid']=[8]*20
        if kind=='iid-fine':target['iid']=[16]*20
        if kind=='icc':target['icc']=[7,14]+[14]*18
        # Every transmitted delta is a legal normative Huffman symbol. The
        # reconstructed index alone is invalid, reproducing that exact stage.
        bits,borders=encode(True,im,1,True,True,False,[target],zero(),32)
        record=store(bits,borders,[target],target,False,True,
            dict(iid_mode=im,icc_mode=1,extension=True),False,im,1)
        record.update(kind=kind,error='PS reconstructed index exceeds quantization grid')
        frames=[];bad_payloads=[]
        for stage in range(3):
            raw=sbr(bits,stage);data=packet(raw);bad_payloads.append(raw.hex())
            frames.append(dict(offset=len(bad_blob),bytes=len(data)));bad_blob.extend(data)
        bad_case=dict(slots=16,bands=64,frames=frames,asc=asc(24000,48000,16,'explicit',ps=True).hex(),pcm_offset=0,samples=12288)
        bad_video=video_fixture([bad_case],bad_blob,channels=2,filename=f'he-aac-ps-invalid-{kind}-synthetic.mp4')
        bad_video.pop('pcm_offset');bad_video.pop('samples')
        record.update(video=bad_video,sbr_payloads=bad_payloads,packet_frames=frames)
        malformed.append(record)
    transition_blob=bytearray();transition_frames=[];transition_payloads=[];transition_expected=[];previous=zero()
    for stage,(im,enabled,count) in enumerate([(0,True,1),(2,True,0),(2,False,0)]):
        rows=[targets(im,im,stage)] if count else []
        bits,borders=encode(True,im,im,enabled,enabled,enabled,rows,previous,32)
        retained=rows[-1] if rows else retain(previous,enabled,enabled,enabled)
        raw=sbr(bits,stage);data=packet(raw);transition_payloads.append(raw.hex())
        transition_frames.append(dict(offset=len(transition_blob),bytes=len(data)));transition_blob.extend(data)
        transition_expected.append(dict(iid_mode=im,icc_mode=im,borders=borders,retained=retained,envelopes=rows))
        previous=retained
    transition_case=dict(slots=16,bands=64,frames=transition_frames,asc=asc(24000,48000,16,'explicit',ps=True).hex(),pcm_offset=0,samples=12288)
    transition_video=video_fixture([transition_case],transition_blob,channels=2,filename='he-aac-ps-mode-transition-synthetic.mp4')
    transition_video.pop('pcm_offset');transition_video.pop('samples');transition_video['pcm_acceptance']='pending owned PS synthesis'
    (DEST/'he-aac-ps-mode-transition-packets.bin').write_bytes(transition_blob)
    (DEST/'aac-ps-history-syntax.bin').write_bytes(binary)
    (DEST/'he-aac-ps-varying-packets.bin').write_bytes(blob)
    (DEST/'he-aac-ps-invalid-packets.bin').write_bytes(bad_blob)
    case=dict(slots=16,bands=64,frames=packet_frames,asc=asc(24000,48000,16,'explicit',ps=True).hex(),pcm_offset=0,samples=12288)
    video=video_fixture([case],blob,channels=2,filename='he-aac-ps-varying-synthetic.mp4')
    video.pop('pcm_offset');video.pop('samples');video['pcm_acceptance']='pending owned PS synthesis'
    manifest=dict(kind='original absolute native PS targets encoded to frequency/time deltas',
                  sha256=hashlib.sha256(binary).hexdigest(),sequences=sequences,
                  video=video,sbr_payloads=payloads,video_targets=video_targets,packet_frames=packet_frames,malformed=malformed,
                  mode_transition=dict(video=transition_video,sbr_payloads=transition_payloads,packet_frames=transition_frames,expected=transition_expected))
    (DEST/'aac-ps-history-oracles.json').write_text(json.dumps(manifest,separators=(',',':'))+'\n')
    with localcontext() as ctx:
        ctx.prec=90
        pi=16*atan(D(1)/5)-4*atan(D(1)/239)
        oracle=dict(source='GOST R 53556.8-2013 numeric grids; phase values by independent Decimal Machin series',
                    iid_coarse=[dict(index=i-7,db=str(v)) for i,v in enumerate(COARSE)],
                    iid_fine=[dict(index=i-15,db=str(v)) for i,v in enumerate(FINE)],
                    icc=[dict(index=i,value=str(D(v))) for i,v in enumerate(ICC)],
                    phase=[dict(index=i,value=str(pi*i/4)) for i in range(8)])
    (DEST/'aac-ps-dequant-oracles.json').write_text(json.dumps(oracle,separators=(',',':'))+'\n')
    print(len(sequences),'original varying PS sequences;',len(binary),'syntax bytes;',len(blob),'AAC packet bytes')

if __name__=='__main__':main()
