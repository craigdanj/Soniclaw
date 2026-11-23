#!/usr/bin/env python3
"""Run identical optimized DSP fixtures against two local source trees.
Not a DAW/driver benchmark. Both builds complete before alternating measurements.
"""
import argparse,csv,json,os,pathlib,platform,statistics,subprocess,tempfile
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--baseline',type=pathlib.Path,required=True)
p.add_argument('--current',type=pathlib.Path,default=pathlib.Path(__file__).resolve().parents[1])
p.add_argument('--output',type=pathlib.Path,required=True)
p.add_argument('--rounds',type=int,default=7)
a=p.parse_args()
if not 3<=a.rounds<=50:p.error('rounds must be 3..50')
a.output.mkdir(parents=True,exist_ok=False)
source=r'''
use soniclaw_core::*;
use soniclaw_dsp::{AutomatedEffect,EffectKind};
use std::{hint::black_box,time::Instant};
fn main(){
 for (name,kind) in [("gain",EffectKind::Gain),("low_pass",EffectKind::LowPass),("state_variable",EffectKind::StateVariable),("rms_compressor",EffectKind::RmsCompressor),("fdn_reverb",EffectKind::FdnReverb)] {
  let mut p=AutomatedEffect::new(kind,"bench").unwrap();
  if matches!(kind,EffectKind::Gain) {p.parameters()[0].set(0.5).unwrap();}
  p.prepare(ProcessSpec{sample_rate:48000.0,max_block_frames:256,channels:2}).unwrap();
  let input:[f32;512]=std::array::from_fn(|i|(i as f32*0.09).sin()*0.1);let mut data=input;
  let ctx=ProcessContext{events:&[],transport:Transport::default()};
  for _ in 0..256 {data.copy_from_slice(&input);p.process(&mut AudioBuffer::new(&mut data,2).unwrap(),&ctx);black_box(&data);}
  let start=Instant::now();
  for _ in 0..4096 {data.copy_from_slice(&input);p.process(&mut AudioBuffer::new(&mut data,2).unwrap(),&ctx);black_box(&data);}
  println!("{},{}",name,start.elapsed().as_secs_f64()*1e9/(4096.0*256.0));
 }
}
'''
env=os.environ.copy();env['RUSTFLAGS']='-C target-cpu=x86-64';env.pop('CARGO_TARGET_DIR',None)
rows=[];binaries={}
with tempfile.TemporaryDirectory(prefix='soniclaw-cpu-') as tmp:
 for label,root in [('baseline',a.baseline.resolve()),('current',a.current.resolve())]:
  d=pathlib.Path(tmp)/label;(d/'src').mkdir(parents=True)
  manifest='[package]\nname="soniclaw-cpu-fixture"\nversion="0.0.0"\nedition="2021"\n[workspace]\n[dependencies]\n'
  for name in ['soniclaw-core','soniclaw-dsp']:manifest+=name+'={path='+json.dumps(str(root/'crates'/name))+'}\n'
  manifest+='[profile.release]\nlto="thin"\ncodegen-units=1\n'
  (d/'Cargo.toml').write_text(manifest);(d/'src/main.rs').write_text(source)
  with (a.output/(label+'-build.log')).open('w') as f:subprocess.run(['cargo','build','--release','--offline','--manifest-path',str(d/'Cargo.toml')],env=env,stdout=f,stderr=subprocess.STDOUT,check=True)
  binaries[label]=d/'target/release/soniclaw-cpu-fixture'
 for round in range(a.rounds):
  for label in (['baseline','current'] if round%2==0 else ['current','baseline']):
   text=subprocess.check_output([str(binaries[label])],env=env,text=True)
   for line in text.strip().splitlines():
    case,value=line.split(',');rows.append({'round':round,'version':label,'case':case,'ns_per_stereo_frame':float(value)})
with (a.output/'callback-fixtures.csv').open('w') as f:
 w=csv.DictWriter(f,fieldnames=list(rows[0]));w.writeheader();w.writerows(rows)
summary={}
for case in sorted({r['case'] for r in rows}):
 med={v:statistics.median(r['ns_per_stereo_frame'] for r in rows if r['case']==case and r['version']==v) for v in binaries}
 summary[case]={**med,'change_percent':(med['current']/med['baseline']-1)*100}
(a.output/'summary.json').write_text(json.dumps({'environment':platform.platform(),'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'rounds':a.rounds,'target_cpu':'x86-64','baseline':str(a.baseline),'current':str(a.current),'measurement':'256-frame stereo blocks, 48 kHz; wall time includes input copy, wrapper and black_box; static controls; 256 warmup/4096 timed blocks per case; alternating binary order; shared Linux VM, no pinning, no driver or DAW','cases':summary},indent=2)+'\n')
print(json.dumps(summary,indent=2))
