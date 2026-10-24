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
    packets = [bytes((j * 13 + i * 41 + 7) % 256 for j in range(40 * shape[3])) for i, shape in enumerate(shapes)]
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
        stbl = atom(b'stbl', atom(b'stsd', word(0, 3) + b''.join(entries[i] for i in table_order))
            + atom(b'stts', word(0, len(selection)) + b''.join(word(240, shapes[i][4]) for i in selection))
            + atom(b'stsz', word(0, 1, 240 * len(selection)))
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
    # Each control table has only the actual description, so legacy admission
    # succeeds and decoding starts from the same fresh predictor state.
    for i in range(3):
        data = movie([i])
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
