"""Explicit generation only: encode authored tone and encode neutral video."""
import hashlib,json,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/matroska-audio-sample-clocks'
def generate():
    OUT.mkdir(parents=True,exist_ok=True)
    cases=[]
    for name,encoder in [('flac','flac'),('alac','alac'),('mp3','libmp3lame'),('mp2','mp2'),('vorbis','vorbis'),('ac3','ac3')]:
        path=OUT/(name+'.mkv')
        subprocess.run(['ffmpeg','-hide_banner','-loglevel','error','-y','-f','lavfi','-i','color=c=gray:s=16x16:r=50:d=0.24','-f','lavfi','-i','aevalsrc=0.25*sin(2*PI*440*t):s=48000:d=0.24','-map','0:v:0','-map','1:a:0','-c:v','libvpx-vp9','-lossless','1','-threads','1','-c:a',encoder,'-strict','experimental','-ar','48000','-ac','2' if name=='vorbis' else '1','-t','0.24',*(['-b:a','128k'] if name=='mp2' else []),str(path)],check=True)
        cases.append({'file':path.name,'encoder':encoder,'sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'240ms authored440Hz mono tone with authored16-square VP9 neutral video. No private input. Generation uses FFmpeg; tests load saved bytes only.','cases':cases},indent=2)+'\n')
if __name__=='__main__':generate()
