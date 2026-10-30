"""Own synthetic stateful ecData blocks; explicit offline generation only.
These are component fixtures, not accepted spatial movies. No private data,
encoder executable, network, or reference implementation is used.
"""
import json
from pathlib import Path
from generate_aac_ld_mps_entropy_fixtures import Writer, field, packed, pcm, quant
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/fixtures/playback-errors/aac-ld-mps-parameters'
BOOKS = ROOT / 'tests/fixtures/playback-errors/aac-ld-mps-entropy/codewords.json'

def maps(bands, stride):
    # Boundary decrement oracle, distinct from runtime closed-form boundaries.
    boundaries = list(range(0, ((bands - 1) // stride + 1) * stride + 1, stride))
    cursor = 0
    while boundaries[-1] > bands:
        cursor = min(cursor + 1, len(boundaries) - 1)
        boundaries[cursor:] = [v - 1 for v in boundaries[cursor:]]
    groups = [list(range(a, b)) for a, b in zip(boundaries, boundaries[1:])]
    widths = [stride] * len(groups)
    for j in range(sum(widths) - bands):
        widths[j % len(widths)] -= 1
    output_groups = []
    start = 0
    for width in widths:
        output_groups.append(list(range(start, start + width)))
        start += width
    return groups, output_groups

def generate():
    books = json.loads(BOOKS.read_text())
    blob = bytearray()
    cases = []
    for kind in ['CLD', 'ICC', 'CPC']:
        default = {'CLD': 15, 'ICC': 0, 'CPC': 10}[kind]
        for bands in range(1, 24):
            for code, stride in enumerate([1, 2, 5, 23]):
                groups, output_groups = maps(bands, stride)
                for initial_coarse in [False, True]:
                    history = [default] * bands
                    old_coarse = False
                    mapped = history[:]
                    mapped_coarse = False
                    frames = []
                    def convert(v, coarse):
                        if coarse:
                            return int(v / 2) if kind == 'CLD' else v // 2
                        return (-15 if v == -7 else 15 if v == 7 else v * 2) if kind == 'CLD' else v * 2
                    def fine(row, coarse):
                        return [convert(v, False) if coarse else v for v in row]
                    def emit(modes, coarse=False, temporal=False, pair=False, independent=False, salt=0, step_code=None):
                        nonlocal history, old_coarse, mapped, mapped_coarse
                        current_code = code if step_code is None else step_code
                        groups, output_groups = maps(bands, [1, 2, 5, 23][current_code])
                        wire = ''.join(field(m, 2) for m in modes)
                        rows = []
                        coded = {}
                        pending = None
                        for i, mode in enumerate(modes):
                            if mode == 0:
                                history, old_coarse = [default] * bands, False
                            elif mode == 3:
                                if pending is not None:
                                    history = pending
                                    pending = None
                                else:
                                    if old_coarse != coarse:
                                        history = [convert(v, coarse) for v in history]
                                    levels, offset, lsb = quant(kind, coarse)
                                    n = len(groups)
                                    previous = [history[g[0]] for g in groups]
                                    if temporal:
                                        # Desired rows derive from alphabet limits, not
                                        # decoder output; ±1 temporal differences also
                                        # expose negative odd CPC requantization.
                                        target = [min(levels - offset - 1, max(-offset, v + (1 if j % 2 else -1))) for j, v in enumerate(previous)]
                                        w = Writer(books)
                                        delta = [((v + offset) >> lsb) - ((h + offset) >> lsb) for v, h in zip(target, previous)]
                                        w.wire = '010'  # Huffman, temporal, 1D
                                        w.vector(kind, True, delta, None, False)
                                        if lsb:
                                            w.wire += ''.join(str((v + offset) & 1) for v in target)
                                        payload = w.wire
                                        compact = [target]
                                    else:
                                        compact = [[(j * 7 + r * 11 + salt) % levels - offset for j in range(n)] for r in range(1 + pair)]
                                        # Ensure both signed CLD endpoints are exercised.
                                        if kind == 'CLD':
                                            compact[0][0] = -offset
                                            if n > 1: compact[0][-1] = levels - offset - 1
                                        payload = '1' + pcm([compact[r][j] + offset for j in range(n) for r in range(1 + pair)], levels)
                                    wire += str(int(pair)) + str(int(coarse)) + field(current_code, 2) + payload
                                    expanded = [[v for v, group in zip(row, groups) for _ in group] for row in compact]
                                    for r, row in enumerate(compact):
                                        coded[i + r] = [v for v, group in zip(row, output_groups) for _ in group]
                                    history = expanded[0]
                                    pending = expanded[1] if pair else None
                                    old_coarse = coarse
                        for i, mode in enumerate(modes):
                            if mode == 0:
                                mapped, mapped_coarse = [default] * bands, False
                            elif mode == 3:
                                mapped, mapped_coarse = coded[i], coarse
                            rows.append(fine(mapped, mapped_coarse))
                        packet = packed(wire)
                        frames.append(dict(offset=len(blob), bytes=len(packet), bits=len(wire), slots=[7, 15] if len(modes) == 2 else [15], frame_slots=16, independent=independent, modes=modes, expected=rows, parsed=history[:], parse_coarse=old_coarse))
                        blob.extend(packet)
                    emit([3, 3], initial_coarse, pair=True, independent=True, salt=2)
                    emit([1, 1])
                    emit([3], not initial_coarse, temporal=True, step_code=(code + 2) % 4)
                    emit([2, 3], initial_coarse, salt=5)
                    emit([0, 3], True, temporal=True, independent=True)
                    emit([3, 0], False, temporal=True, step_code=(code + 2) % 4)
                    emit([1])
                    emit([0], independent=True)
                    cases.append(dict(name=f'{kind}-{bands}-stride{stride}-coarse{int(initial_coarse)}', kind=kind, bands=bands, default=default, groups=groups, output_groups=output_groups, frames=frames))
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / 'packets.bin').write_bytes(blob)
    (OUT / 'manifest.json').write_text(json.dumps(dict(scope='Parameter state component only; no spatial movie/playback acceptance.', cases=cases), separators=(',', ':')) + '\n')
    print(len(cases), 'sequences,', sum(len(c['frames']) for c in cases), 'blocks,', len(blob), 'packet bytes')
if __name__ == '__main__': generate()
