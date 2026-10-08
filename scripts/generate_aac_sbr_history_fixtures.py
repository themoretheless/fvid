#!/usr/bin/env python3
"""Original four-frame SBR coefficient history/Decimal dequantization vectors.

Numeric protocol coverage, not encoded HE-AAC PCM acceptance. Saved bit
payloads exercise the actual owned extension/CRC/data parser. References use
64-bin expansion rather than the Rust previous-band lookup; noise uses indices.
No private media, foreign encoder/decoder, FFmpeg or network.
"""
from decimal import Decimal as D, localcontext
from pathlib import Path
import hashlib
import json
from generate_aac_sbr_data_fixtures import field, word, grid
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_sbr_extension_fixtures import crc

DEST = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'

def row(values, temporal, coarse, balance, noise):
    if noise: tb,fb,width = (9,7,5) if balance else (8,5,5)
    else:
        tb,fb,width = ((6,7,5) if coarse else (2,3,6)) if balance else ((4,5,6) if coarse else (0,1,7))
    result = '' if temporal else field(values[0],width)
    return result + ''.join(word(tb if temporal else fb,v) for v in values[int(not temporal):])

def header(amplitude, crossover):
    return field(int(amplitude),1)+'0000'+'1110'+field(crossover,3)+'00'+'10'+'00'+'0'+'11'

def continuous_grid(kind):
    bits, expected = grid(kind)
    if kind == 'varvar':
        # Keep consecutive frame boundaries continuous in these sequences.
        bits = bits[:2] + '0000' + bits[6:]
        expected.update(leading_offset=0, trailing_offset=0)
    return bits, expected

def sequence(kind, slots, crossover, poison_noise=False):
    master,high,low,noise_borders = tables(7,14,0,False,crossover,3)
    count = 1 if kind == 'mono' else 2
    coupled = kind == 'coupled'
    expanded = [[None]*64 for _ in range(count)]
    last_noise = [None]*count
    frames = []
    for frame_index, grid_name in enumerate(['fixvar','varfix','varvar','fixfix1']):
        amplitude = frame_index in [1,2]
        header_present = frame_index != 2
        protected = frame_index%2 != 0
        g0,e0 = continuous_grid(grid_name)
        # Uncoupled grids differ, including envelope counts/resolutions.
        g1,e1 = continuous_grid(['varvar','fixvar','varfix','fixfix2'][frame_index])
        grids = [e0] if count == 1 else [e0,e0 if coupled else e1]
        payload = '0' + (field(int(coupled),1) if count == 2 else '') + g0
        if count==2 and not coupled: payload += g1
        channels = []
        for ch,g in enumerate(grids):
            env_flags = [frame_index!=0 or i!=0 for i in range(len(g['high_resolution']))]
            noise_flags = [frame_index!=0 or i!=0 for i in range(1 if len(env_flags)==1 else 2)]
            payload += ''.join(field(int(v),1) for v in env_flags+noise_flags)
            channels.append(dict(coarse=amplitude and g['class_name']!='FixFix' or amplitude and len(env_flags)!=1,
                                 quantized_envelope=[],quantized_noise=[],envelope=[],noise=[],
                                 env_flags=env_flags,noise_flags=noise_flags))
        nq = len(noise_borders)-1
        for ch in range(1 if coupled else count):
            payload += ''.join(field((i+ch+frame_index)%4,2) for i in range(nq))
        env_bits,noise_bits = [],[]
        for ch,(g,expected) in enumerate(zip(grids,channels)):
            balance = coupled and ch==1
            multiplier = 2 if balance else 1
            text = ''
            for env,(is_high,temporal) in enumerate(zip(g['high_resolution'],expected['env_flags'])):
                borders = high if is_high else low
                values = [(i+env+ch+frame_index)%3-1 for i in range(len(borders)-1)]
                if not temporal: values[0] = 12 if balance else 20+ch
                text += row(values,temporal,expected['coarse'],balance,False)
                quantized = []
                for i,delta in enumerate(values):
                    base = expanded[ch][borders[i]] if temporal else (quantized[-1] if i else 0)
                    assert base is not None
                    quantized.append(base+multiplier*delta)
                # Expand to physical QMF bins, independently of Rust's lookup.
                for i,value in enumerate(quantized):
                    for k in range(borders[i],borders[i+1]): expanded[ch][k] = value
                expected['quantized_envelope'].append(quantized)
            env_bits.append(text)
            text = ''
            for env,temporal in enumerate(expected['noise_flags']):
                values = [(i+env+ch+frame_index)%3-1 for i in range(nq)]
                if not temporal: values[0] = 5 if balance else 12+ch
                if poison_noise and frame_index==0 and ch==count-1 and env==0:
                    values[0] = 31 # syntactically valid five-bit value, outside normative Q range
                text += row(values,temporal,False,balance,True)
                quantized = []
                for i,delta in enumerate(values):
                    base = last_noise[ch][i] if temporal else (quantized[-1] if i else 0)
                    quantized.append(base+multiplier*delta)
                last_noise[ch] = quantized
                expected['quantized_noise'].append(quantized)
            noise_bits.append(text)
        payload += ''.join(e+n for e,n in zip(env_bits,noise_bits)) if coupled else ''.join(env_bits)+''.join(noise_bits)
        payload += '0'*count + '0' # harmonic flags and no extended data
        body = field(int(header_present),1)+(header(amplitude,crossover) if header_present else '')+payload
        fill_count = (-(4+10*int(protected)+len(body)))%8
        body += '1011010'[:fill_count]
        checksum = crc(body) if protected else None
        bits = field(14 if protected else 13,4)+(field(checksum,10) if protected else '')+body
        assert len(bits)%8==0
        raw = bytes(int(bits[p:p+8],2) for p in range(0,len(bits),8))
        if coupled:
            for levels,balances in zip(channels[0]['quantized_envelope'],channels[1]['quantized_envelope']):
                a = D(1 if channels[0]['coarse'] else 2); pan = D(12 if channels[0]['coarse'] else 24)
                numerator = [D(2)**(7+D(v)/a) for v in levels]
                channels[0]['envelope'].append([str(t/(1+D(2)**((pan-D(b))/a))) for t,b in zip(numerator,balances)])
                channels[1]['envelope'].append([str(t/(1+D(2)**((D(b)-pan)/a))) for t,b in zip(numerator,balances)])
            for levels,balances in zip(channels[0]['quantized_noise'],channels[1]['quantized_noise']):
                numerator = [D(2)**(7-v) for v in levels]
                channels[0]['noise'].append([str(t/(1+D(2)**(b-12))) for t,b in zip(numerator,balances)])
                channels[1]['noise'].append([str(t/(1+D(2)**(12-b))) for t,b in zip(numerator,balances)])
        else:
            for ch in channels:
                a = D(1 if ch['coarse'] else 2)
                ch['envelope'] = [[str(D(2)**(6+D(v)/a)) for v in values] for values in ch['quantized_envelope']]
                ch['noise'] = [[str(D(2)**(6-v)) for v in values] for values in ch['quantized_noise']]
        for ch in channels:
            del ch['env_flags']; del ch['noise_flags']
        frames.append((raw,dict(amplitude=amplitude,header_present=header_present,crc=checksum,channels=channels)))
    return frames

def main():
    sequences = []; binary = bytearray()
    with localcontext() as context:
        context.prec = 80
        for kind in ['mono','uncoupled','coupled']:
            for slots in [15,16]:
                for crossover in [0,1]:
                    frames = []
                    for raw,expected in sequence(kind,slots,crossover):
                        expected.update(offset=len(binary),byte_length=len(raw));binary.extend(raw);frames.append(expected)
                    invalid = sequence(kind,slots,crossover,poison_noise=True)[0][0]
                    failure = dict(offset=len(binary),byte_length=len(invalid))
                    binary.extend(invalid)
                    sequences.append(dict(kind=kind,slots=slots,crossover=crossover,frames=frames,invalid_noise=failure))
    (DEST/'aac-sbr-history-syntax.bin').write_bytes(binary)
    (DEST/'aac-sbr-history-decimal.json').write_text(json.dumps(dict(
        precision=80,kind='original bit syntax and coefficient/DSP vectors, not encoded HE-AAC PCM acceptance',
        sha256=hashlib.sha256(binary).hexdigest(),sequences=sequences),separators=(',',':'))+'\n')
    print(f'{len(sequences)} four-frame sequences, {len(binary)} bytes, sha256 {hashlib.sha256(binary).hexdigest()}')

if __name__ == '__main__': main()
