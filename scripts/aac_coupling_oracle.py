"""Independent direct-cosine oracle for the hand-authored sine-window CCE fixture.

No AAC decoder, FFT, FVid synthesis or reference executable is used. This is
specific to the generator's +/-1 four-tuples, gain codes and first-order TNS.
"""
import math
import struct


def pcm(args):
    channels=2 if args.stereo else 1
    overlaps=[[0.0]*1024 for _ in range(channels)]
    output=bytearray()
    for frame in range(6):
        sequence=(1 if frame==0 else 3 if frame==5 else 2) if args.short else 0
        n=128 if sequence==2 else 1024
        spectra=[]
        for window in range(8 if sequence==2 else 1):
            coefficients=[0.0]*n
            for band in ([0,2] if args.multiband else [0]):
                for k in range(band*4,band*4+4):
                    coefficients[k]=1024.0*(1 if (frame+window+band)%2==0 else -1)
            spectra.append(coefficients)
        signals=[]
        for channel in range(channels):
            active=not args.stereo or args.selection in ['shared','separate'] or (args.selection=='left' and channel==0) or (args.selection=='right' and channel==1)
            windows=[]
            for window,source in enumerate(spectra):
                values=source[:]
                if not active:
                    values=[0.0]*n
                elif args.stereo and args.selection=='separate' and channel==1:
                    if args.band_gain:
                        gains=[-2**(-2/8),2**(-1/8)] if args.signed_gain else [2**(-4/8),2**(-1/8)]
                        if args.split_groups and sequence==2 and window>=4:
                            gains=[2**(-1/8),-2**(-1/8)]
                        for band,gain in zip([0,2] if args.multiband else [0],gains):
                            for k in range(band*4,band*4+4): values[k]*=gain
                    else:
                        values=[v*2**(-4/8) for v in values]
                if args.point=='before-tns':
                    # Encoded reflection index +1, resolution 3, forward order 1.
                    previous=0.0
                    for k in range(12 if args.multiband else 4):
                        values[k]-=math.sin(math.pi/7)*previous
                        previous=values[k]
                nonzero=[(k,v) for k,v in enumerate(values) if v]
                transformed=[sum(v*math.cos(math.pi/n*(t+0.5+n/2)*(k+0.5)) for k,v in nonzero)*2/n/65536 for t in range(2*n)]
                windows.append(transformed)
            block=[0.0]*2048
            if sequence==2:
                for window,values in enumerate(windows):
                    for i,value in enumerate(values):
                        block[448+window*128+i]+=value*math.sin(math.pi/256*(i+0.5))
            else:
                for i,value in enumerate(windows[0]):
                    weight=math.sin(math.pi/2048*(i+0.5))
                    if sequence==1 and i>=1024:
                        j=i-1024
                        weight=1.0 if j<448 else math.sin(math.pi/256*(j-448+128+0.5)) if j<576 else 0.0
                    if sequence==3 and i<1024:
                        weight=0.0 if i<448 else math.sin(math.pi/256*(i-448+0.5)) if i<576 else 1.0
                    block[i]=value*weight
            signals.append([block[i]+overlaps[channel][i] for i in range(1024)])
            overlaps[channel]=block[1024:]
        for i in range(1024):
            for channel in range(channels): output+=struct.pack('<f',signals[channel][i])
    return output
