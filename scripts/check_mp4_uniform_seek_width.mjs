// Explicit wasm32 regression, separate from ordinary offline/native tests.
import {readFileSync} from 'node:fs';
Error.stackTraceLimit=50;
const {instance}=await WebAssembly.instantiate(readFileSync(process.argv[2]),{});
if(instance.exports.pointer_width()!==32)throw new Error('Expected actual wasm32 pointers');
const checks=[];
for(let c=0;c<4;c++){
 const delta=BigInt(c%2===0?20:960);
 for(const target of [-1n,0n,delta-1n,delta,2n*delta,3n*delta,1n<<32n,(1n<<32n)+delta,1n<<40n,(1n<<40n)+delta,(1n<<63n)-1n]){
  const want=Number(target<0n?0n:target/delta>2n?2n:target/delta);
  const got=instance.exports.sample_at(c,target);
  checks.push({case:c,target:String(target),want,got,pass:got===want});
 }
}
for(const count of [0,1,3,100])for(const duration of [0,1,20,0xffffffff]){
 for(const target of [-1n,0n,1n,19n,20n,21n,0xffffffffn,1n<<32n,(1n<<32n)+20n,1n<<40n,(1n<<63n)-1n]){
  const last=BigInt(Math.max(0,count-1));
  const quotient=duration===0?0n:(target<0n?0n:target)/BigInt(duration);
  const want=Number(quotient>last?last:quotient);
  const got=instance.exports.synthetic_at(count,duration,target);
  checks.push({count,duration,target:String(target),want,got,pass:got===want});
 }
}
console.log(JSON.stringify({pointerWidth:32,checks,passed:checks.filter(c=>c.pass).length,failed:checks.filter(c=>!c.pass).length},null,2));
if(checks.some(c=>!c.pass))process.exitCode=1;
