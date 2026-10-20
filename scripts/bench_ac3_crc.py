#!/usr/bin/env python3
"""Paired optimized actual-native-codec CRC experiment; no media generation/network.

Only audio API type shims are used to compile the unchanged production decoder
body standalone, avoiding GPU/app compilation. This is not a packaged release
benchmark. Both variants must produce identical full PCM digests before timing.
"""
import argparse,hashlib,json,os,re,statistics,subprocess,tempfile,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
BITWISE='''fn frame_crc_valid(frame: &[u8]) -> bool {
    let mut crc = 0u16;
    for &byte in frame.get(2..).unwrap_or_default() {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = (crc << 1) ^ if crc & 0x8000 != 0 { 0x8005 } else { 0 };
        }
    }
    frame.len() >= 4 && crc == 0
}
'''
TABLE='''const AC3_CRC_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = (index as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc << 1) ^ if crc & 0x8000 != 0 { 0x8005 } else { 0 };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
};
fn frame_crc_valid(frame: &[u8]) -> bool {
    let mut crc = 0u16;
    for &byte in frame.get(2..).unwrap_or_default() {
        crc = (crc << 8) ^ AC3_CRC_TABLE[usize::from((crc >> 8) as u8 ^ byte)];
    }
    frame.len() >= 4 && crc == 0
}
'''
HARNESS='''#![allow(dead_code)]
pub type Result<T> = std::result::Result<T,String>;
pub fn invalid(s:&str)->String{s.to_owned()}
pub mod audio {
    pub enum SampleFormat {F32}
    pub struct AudioSpec {pub sample_rate:u32,pub channels:u16,pub format:SampleFormat}
    pub struct AudioPacket {pub data:Vec<u8>,pub pts:u64,pub timebase_num:u32,pub timebase_den:u32}
    pub trait AudioDecode {fn decode_encoded(&mut self,data:&[u8],pts:u64,duration:u64)->crate::Result<Option<AudioPacket>>;fn reset(&mut self);}
}
#[path="CODEC_PATH"]mod ac3;
use audio::AudioDecode;
fn digest(bytes:&[u8])->u64{bytes.iter().fold(0xcbf29ce484222325u64,|h,b|(h^u64::from(*b)).wrapping_mul(0x100000001b3))}
fn main(){
    let args=std::env::args().collect::<Vec<_>>();let data=std::fs::read(&args[1]).unwrap();let rate=args[2].parse::<u32>().unwrap();let channels=args[3].parse::<u16>().unwrap();let iterations=args[4].parse::<u32>().unwrap();
    let mut decoder=ac3::Ac3Decoder::new(&[],rate,channels).unwrap();
    let reference=decoder.decode_encoded(&data,0,0).unwrap().unwrap();let hash=digest(&reference.data);let length=reference.data.len();
    if let Some(path)=args.get(5){std::fs::write(path,&reference.data).unwrap();}
    for _ in 0..50 {decoder.reset();std::hint::black_box(decoder.decode_encoded(std::hint::black_box(&data),0,0).unwrap().unwrap());}
    let started=std::time::Instant::now();for _ in 0..iterations {decoder.reset();std::hint::black_box(decoder.decode_encoded(std::hint::black_box(&data),0,0).unwrap().unwrap());}
    println!("{} {} {}",started.elapsed().as_nanos(),hash,length);
}
'''
def variant(source,replacement):
    begin=source.index('fn frame_crc_valid(')
    const=source.rfind('const AC3_CRC_TABLE:',0,begin)
    if const!=-1:begin=const
    end=source.index('impl AudioDecode for Ac3Decoder {',begin)
    return source[:begin]+replacement+'\n'+source[end:]
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--pairs',type=int,default=9);ap.add_argument('--iterations',type=int,default=2000);ap.add_argument('--output',type=Path,required=True);args=ap.parse_args()
    assert args.pairs>=3 and args.iterations>0
    source=(ROOT/'src/codec/ac3_decoder.rs').read_text();report=dict(source_sha256=hashlib.sha256(source.encode()).hexdigest(),compiler=subprocess.check_output(['rustc','--version'],text=True).strip(),flags=['--edition=2024','-O','-C','target-cpu=native'],scope='actual production native decoder with standalone audio API type shims; repeated reset/full-stream decode; no app/GPU/packaged release claim',pairs=args.pairs,iterations=args.iterations,raw=[],verification=[])
    workloads=[('stereo',ROOT/'tests/fixtures/audio/ac3-stereo.ac3',48000,2),('coupled',ROOT/'tests/fixtures/audio/ac3-coupled.ac3',48000,2)]
    with tempfile.TemporaryDirectory(prefix='fvid-ac3-crc-bench-') as directory:
        tmp=Path(directory);bins={}
        for name,replacement in [('bitwise',BITWISE),('table',TABLE)]:
            codec=tmp/(name+'-codec.rs');codec.write_text(variant(source,replacement));harness=tmp/(name+'.rs');harness.write_text(HARNESS.replace('CODEC_PATH',str(codec)));binary=tmp/name
            subprocess.run(['rustc',str(harness),*report['flags'],'-o',str(binary)],check=True);bins[name]=binary
        def run(name,path,rate,channels,iterations):
            ns,hash_value,length=map(int,subprocess.check_output([str(bins[name]),str(path),str(rate),str(channels),str(iterations)],text=True).split());return dict(ns=ns,hash=hash_value,bytes=length)
        controls=workloads+[('surround',ROOT/'tests/fixtures/audio/ac3-51.ac3',48000,6),('mono32',ROOT/'tests/fixtures/audio/ac3-mono-32k.ac3',32000,1)]+[(name,ROOT/'tests/fixtures/playback-errors/ac3-crc'/f'{name}.ac3',48000,2)for name in ['crc1','crc2','payload']]
        for label,path,rate,channels in controls:
            outputs=[run(name,path,rate,channels,1)for name in ['bitwise','table']];assert outputs[0]['hash']==outputs[1]['hash'] and outputs[0]['bytes']==outputs[1]['bytes']
            pcm=[]
            for name in ['bitwise','table']:
                saved=tmp/(name+'-'+label+'.pcm');subprocess.check_output([str(bins[name]),str(path),str(rate),str(channels),'1',str(saved)]);pcm.append(saved.read_bytes())
            assert pcm[0]==pcm[1],label
            report['verification'].append(dict(name=label,input_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),pcm_sha256=hashlib.sha256(pcm[0]).hexdigest(),bytes=outputs[0]['bytes']))
        for pair in range(args.pairs):
            for label,path,rate,channels in workloads:
                order=['bitwise','table']if pair%2==0 else ['table','bitwise'];results={name:run(name,path,rate,channels,args.iterations)for name in order};assert results['bitwise']['hash']==results['table']['hash'] and results['bitwise']['bytes']==results['table']['bytes'];ratio=results['table']['ns']/results['bitwise']['ns'];report['raw'].append(dict(pair=pair,workload=label,order=order,results=results,ratio=ratio));print(f'pair{pair} {label}: table/bitwise={ratio:.6f}',flush=True)
    report['summary']={label:dict(median_ratio=statistics.median(r['ratio']for r in report['raw']if r['workload']==label),ratios=[r['ratio']for r in report['raw']if r['workload']==label])for label,*_ in workloads};args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['summary']),flush=True)
if __name__=='__main__':main()
