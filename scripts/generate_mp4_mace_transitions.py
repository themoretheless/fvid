"""Own MACE blocks and VP9 companion, with sample-counted QuickTime tables."""
import json, struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word
from generate_mp4_fragment_description_regressions import atoms
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-mace-transitions'

def generate():
    video = dict(atoms(mp4([frame(8, '420', 16, 16)] * 6, 8, '420', (16, 16), list(range(1, 7)))))
    header = atom(b'ftyp', video[b'ftyp'])
    shapes = [(b'MAC3', 8000, 1, 2, 6), (b'MAC6', 16000, 2, 2, 3), (b'MAC3', 8000, 2, 4, 6)]
    shapes.append((b'sowt',8000,1,2,6))
    shapes.append((b'MAC6',8000,1,1,6))
    packets = [bytes((j * 13 + i * 41 + 7) % 256 for j in range(40 * shape[3])) for i, shape in enumerate(shapes[:3])]
    packets.append(b''.join(struct.pack('<h',((j*89)%20000)-10000) for j in range(240)))
    packets.append(bytes((j*29+17)%256 for j in range(40)))
    entries = []
    for codec, rate, channels, _, _ in shapes:
        e = bytearray(28); e[6:8] = struct.pack('>H', 1)
        e[16:20] = struct.pack('>HH', channels, 16); e[24:28] = word(rate << 16)
        entries.append(atom(codec, e))
    def movie(selection, table_order=None):
        table_order = table_order or [0, 1, 2]
        offsets = []; current = len(header) + 8 + len(video[b'mdat'])
        for i in selection: offsets.append(current); current += len(packets[i])
        duration = sum(240 * shapes[i][4] for i in selection)
        stbl = atom(b'stbl', atom(b'stsd', word(0, len(table_order)) + b''.join(entries[i] for i in table_order))
            + atom(b'stts', word(0, len(selection)) + b''.join(word(240, shapes[i][4]) for i in selection))
            + atom(b'stsz', word(0, 0, 240 * len(selection), *[2 if i==3 else 1 for i in selection for _ in range(240)]))
            + atom(b'stsc', word(0, len(selection)) + b''.join(word(j+1, 240, table_order.index(i)+1) for j,i in enumerate(selection)))
            + atom(b'stco', word(0, len(selection), *offsets)))
        tkhd = bytearray(84); tkhd[:4] = word(3); tkhd[12:16] = word(2)
        tkhd[20:24] = word((duration * 50 + 47999)//48000)
        tkhd[40:44] = tkhd[56:60] = word(65536); tkhd[72:76] = word(0x40000000)
        mdhd = bytearray(24); mdhd[12:20] = word(48000, duration)
        track = atom(b'trak', atom(b'tkhd', tkhd) + atom(b'mdia', atom(b'mdhd', mdhd)
            + atom(b'hdlr', word(0,0)+b'soun'+bytes(12))
            + atom(b'minf', atom(b'dinf', atom(b'dref', word(0,1)+atom(b'url ',word(1))))+stbl)))
        return header + atom(b'mdat',video[b'mdat']+b''.join(packets[i] for i in selection)) + atom(b'moov',video[b'moov']+track)
    files = {'changes.mp4': movie([0,1,2,0]), 'start-third.mp4': movie([0,1,2,0], [2,1,0])}
    def fragmented(hybrid=False, reversed_table=False, mixed=False):
        order=([2,1,3,0] if reversed_table else [0,1,2,3]) if mixed else ([2,1,0] if reversed_table else [0,1,2])
        selection=[0,3,1,2,0] if mixed else [0,1,2,0]
        original=dict(atoms(movie([0] if hybrid else selection,order)))
        def init(body,audio=False):
            out=b''
            for k,b in atoms(body):
                if k==b'trak':b=init(b,b'soun' in b)
                elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=init(b,audio)
                elif audio and not hybrid:
                    if k in [b'stts',b'stsc',b'stco']:b=word(0,0)
                    elif k==b'stsz':b=word(0,0,0)
                out+=atom(k,b)
            return out
        brand=bytearray(video[b'ftyp']);brand[:4]=b'iso5';brand[8:12]=b'iso5'
        initial=video[b'mdat']+(packets[0] if hybrid else b'')
        movie_body=init(original[b'moov'])+atom(b'mvex',atom(b'trex',word(0,2,order.index(0)+1,36,2,0x02000000)))
        result=atom(b'ftyp',brand)+atom(b'mdat',initial)+atom(b'moov',movie_body)
        dts=1440 if hybrid else 0
        for seq,identity in enumerate(selection[1:] if hybrid else selection,1):
            size=shapes[identity][3];step=shapes[identity][4]*(1 if identity==3 else 6)
            samples=240 if identity==3 else 40
            h=atom(b'tfhd',word(0x02003a,2,order.index(identity)+1,step,size,0x02000000))
            def build(offset):
                return atom(b'moof',atom(b'mfhd',word(0,seq))+atom(b'traf',h+atom(b'tfdt',word(0,dts))+atom(b'trun',word(1,samples,offset))))
            m=build(0);result+=build(len(m)+8)+atom(b'mdat',packets[identity]);dts+=step*samples
        return result
    files['mixed.mp4']=movie([0,3,1,2,0],[0,1,2,3])
    files['mixed-third.mp4']=movie([0,3,1,2,0],[2,1,3,0])
    files['mixed-fragmented.mp4']=fragmented(mixed=True)
    files['mixed-hybrid.mp4']=fragmented(hybrid=True,mixed=True)
    files['fragmented.mp4']=fragmented()
    files['fragmented-third.mp4']=fragmented(reversed_table=True)
    files['hybrid.mp4']=fragmented(hybrid=True)
    # Each control table has only the actual description, so legacy admission
    # succeeds and decoding starts from the same fresh predictor state.
    for i in range(4):
        data = movie([i], [0,1,2,3])
        def single(body, audio=False):
            out = b''
            for k,b in atoms(body):
                if k==b'trak':b=single(b,b'soun' in b)
                elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=single(b,audio)
                elif k==b'stsd' and b'MAC' in b:b=word(0,1)+entries[i]
                elif audio and k==b'stsc' and struct.unpack_from('>I',b,4)[0]==1:b=word(0,1,1,240,1)
                out+=atom(k,b)
            return out
        files[f'control-{i}.mp4'] = single(data)
    from generate_mp4_variable_rate_edits import edit_movie
    files['control-6mono.mp4']=movie([4],[4])
    def grouped_mono6():
        original=dict(atoms(files['control-6mono.mp4']))
        def empty(body,audio=False):
            out=b''
            for k,b in atoms(body):
                if k==b'trak':b=empty(b,b'soun' in b)
                elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=empty(b,audio)
                elif audio and k in [b'stts',b'stsc',b'stco']:b=word(0,0)
                elif audio and k==b'stsz':b=word(0,0,0)
                out+=atom(k,b)
            return out
        brand=bytearray(video[b'ftyp']);brand[:4]=b'iso5';brand[8:12]=b'iso5'
        result=atom(b'ftyp',brand)+atom(b'mdat',video[b'mdat'])+atom(b'moov',empty(original[b'moov'])+atom(b'mvex',atom(b'trex',word(0,2,1,0,0,0x02000000))))
        start=0;dts=0
        for seq,count in enumerate([1,7,32],1):
            data=packets[4][start:start+count];start+=count;duration=count*36
            h=atom(b'tfhd',word(0x02003a,2,1,duration,len(data),0x02000000))
            def moof(offset):return atom(b'moof',atom(b'mfhd',word(0,seq))+atom(b'traf',h+atom(b'tfdt',word(0,dts))+atom(b'trun',word(1,1,offset))))
            m=moof(0);result+=moof(len(m)+8)+atom(b'mdat',data);dts+=duration
        return result
    files['control-6mono-grouped.mp4']=grouped_mono6()
    for name in ['control-0','control-1','control-2','control-6mono','control-6mono-grouped']:
        files[name+'-edits.mp4']=edit_movie(files[name+'.mp4'],[(3,-1),(8,192),(2,-1),(8,192)])
    files['fragmented-edits.mp4']=edit_movie(files['fragmented.mp4'],[(5,-1),(65,480),(5,-1),(65,480)])
    files['hybrid-edits.mp4']=edit_movie(files['hybrid.mp4'],[(5,-1),(65,480),(5,-1),(65,480)])
    files['mixed-fragmented-edits.mp4']=edit_movie(files['mixed-fragmented.mp4'],[(5,-1),(65,480),(5,-1),(65,480)])
    files['mixed-edits.mp4']=edit_movie(files['mixed.mp4'],[(5,-1),(65,480),(5,-1),(65,480)])
    files['edits.mp4']=edit_movie(files['changes.mp4'],[(5,-1),(65,480),(5,-1),(65,480)])
    def composition(body, audio=False):
        out=b''
        for k,b in atoms(body):
            if k==b'trak':b=composition(b,b'soun' in b)
            elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=composition(b,audio)
            if audio and k==b'stbl':b+=atom(b'ctts',word(1<<24,1,960)+struct.pack('>i',-36))
            out+=atom(k,b)
        return out
    files['negative.mp4']=composition(files['changes.mp4'])
    def patch_audio(body, kind, position, value, audio=False):
        out=b''
        for k,b in atoms(body):
            if k==b'trak':b=patch_audio(b,kind,position,value,b'soun' in b)
            elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=patch_audio(b,kind,position,value,audio)
            elif audio and k==kind:
                b=bytearray(b);b[position:position+4]=word(value)
            out+=atom(k,b)
        return out
    files['bad-partial-block.mp4']=patch_audio(files['changes.mp4'],b'stsc',12,239)
    files['bad-outside-mdat.mp4']=patch_audio(files['changes.mp4'],b'stco',20,0xfffffff0)
    files['bad-composition-block.mp4']=patch_audio(files['negative.mp4'],b'ctts',8,959)
    files['manifest.json'] = (json.dumps(dict(provenance='Owned arithmetic MACE bytes and neutral VP9. No private media, external codec, FFmpeg or network.',selection=[0,1,2,0],shapes=[dict(codec=s[0].decode(),rate=s[1],channels=s[2],block_bytes=s[3],sample_ticks=s[4])for s in shapes]),indent=2)+'\n').encode()
    return files
if __name__ == '__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
