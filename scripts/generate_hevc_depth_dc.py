"""Own Annex I depth-DC CABAC component streams, offline and deterministic."""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/fixtures/playback-errors/hevc-depth-dc'

class Encoder:
    def __init__(self, init, qp):
        source = (ROOT / 'src/codec/cabac_tables.rs').read_text()
        def table(name):
            text = source.split('const ' + name + ':', 1)[1].split('= ', 1)[1].split(';', 1)[0]
            return list(map(int, re.findall(r'\d+', text)))
        self.lps, self.tm, self.tl = [table(n) for n in ['RANGE_LPS', 'TRANS_MPS', 'TRANS_LPS']]
        self.contexts = []
        for value in [[0, 0, 64][init], 154, 154]:
            pre = max(1, min(126, (((value >> 4) * 5 - 45) * qp >> 4) + (value & 15) * 8 - 16))
            self.contexts.append([63 - pre if pre <= 63 else pre - 64, int(pre > 63)])
        self.low, self.range, self.width = 0, 510, 9
    def decision(self, index, bit):
        state, mps = self.contexts[index]
        lps = self.lps[state * 4 + ((self.range >> 6) & 3)]
        self.range -= lps
        if bit != mps:
            self.low += self.range
            self.range = lps
            self.contexts[index] = [self.tl[state], mps ^ int(state == 0)]
        else:
            self.contexts[index] = [self.tm[state], mps]
        while self.range < 256:
            self.low *= 2
            self.range *= 2
            self.width += 1
    def bypass(self, bit):
        self.low = self.low * 2 + (self.range if bit else 0)
        self.width += 1
    def absolute(self, value):
        for i in range(3):
            self.decision(1, value > i)
            if value <= i:
                return
        code = value - 2
        order = code.bit_length() - 1
        for _ in range(order):
            self.bypass(1)
        self.bypass(0)
        for shift in reversed(range(order)):
            self.bypass((code >> shift) & 1)
    def finish(self):
        code = self.low + self.range - 1
        padding = (-self.width) % 8
        return (code << padding).to_bytes((self.width + padding) // 8, 'big')

def generate():
    files, cases = {}, []
    for slice_name, swap, init in [('I', False, 0), ('P', False, 1), ('B', False, 2), ('P', True, 2), ('B', True, 1)]:
        for qp in [0, 26, 51]:
            encoder = Encoder(init, qp)
            blocks = []
            for dim in [False, True]:
                for intra, dc_only in [(False, True), (True, True), (True, False)]:
                    if not dim and not dc_only:
                        continue
                    for present in ([False, True] if intra and dc_only else [True]):
                        for value in [0, 1, 2, 3, 4, 6, 7, 15, 255, 65535]:
                            for sign in [0, 1]:
                                encoder.decision(2, dc_only)
                                if intra and dc_only:
                                    encoder.decision(0, present)
                                segments = 2 if dim else 1
                                expected = []
                                for _ in range(segments):
                                    magnitude = (value if present else 0) + 2 - segments
                                    if present:
                                        encoder.absolute(value)
                                        if magnitude:
                                            encoder.bypass(sign)
                                    expected.append(-magnitude if present and sign else magnitude)
                                blocks.append(dict(intra=intra, dc_only=dc_only, dim=dim, offsets=expected + [0] * (2 - segments)))
            cu_blocks = []
            for mask in range(16):
                dim = [bool(mask & (1 << i)) for i in range(4)]
                offsets = [[0, 0] for _ in range(4)]
                for i in range(4):
                    if dim[i]:
                        for segment in range(2):
                            value = 2 * i + segment
                            encoder.absolute(value)
                            if value:
                                encoder.bypass(segment != 0)
                            offsets[i][segment] = -value if segment else value
                cu_blocks.append(dict(dim=dim, offsets=offsets))
            name = f'{slice_name}-swap{int(swap)}-qp{qp}.cabac'
            files[name] = encoder.finish()
            cases.append(dict(file=name, slice=slice_name, swap=swap, qp=qp, blocks=blocks, cu_blocks=cu_blocks))
    files['manifest.json'] = (json.dumps(dict(provenance='Own arbitrary-precision interval encoder and authored Annex I depth DC syntax. Component streams, not full video acceptance.', cases=cases), indent=2) + '\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
