#!/usr/bin/env python3
"""Run reproducible source checks and optionally external format validators."""
import argparse,json,pathlib,shutil,subprocess,sys,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',type=pathlib.Path,default=ROOT/'validation-local');p.add_argument('--clap-validator',type=pathlib.Path);p.add_argument('--vst3-validator',type=pathlib.Path);p.add_argument('--vst3',type=pathlib.Path);args=p.parse_args();args.output.mkdir(parents=True,exist_ok=True);cargo=shutil.which('cargo')or pathlib.Path.home()/'.cargo/bin/cargo';commands=[('fmt',[cargo,'fmt','--all','--check']),('clippy',[cargo,'clippy','--workspace','--all-targets','--all-features','--locked','--','-D','warnings']),('tests',[cargo,'test','--workspace','--all-features','--locked']),('docs',[cargo,'doc','--workspace','--all-features','--no-deps','--locked'])]
 if args.clap_validator:
  commands+=[('reference-build',[cargo,'build','--release','-p','soniclaw-reference','--locked'])]
  lib=ROOT/'target/release'/('soniclaw_reference.dll'if sys.platform=='win32'else'libsoniclaw_reference.dylib'if sys.platform=='darwin'else'libsoniclaw_reference.so')
  commands+=[('clap-validator',[args.clap_validator.resolve(),'validate','--hide-output','--json',lib])]
 if args.vst3_validator:
  if not args.vst3:p.error('--vst3-validator requires --vst3 bundle-path')
  commands+=[('vst3-validator',[args.vst3_validator.resolve(),args.vst3.resolve()])]
 result=[]
 for name,command in commands:
  start=time.monotonic();log=args.output/(name+'.log')
  with log.open('w')as f:run=subprocess.run(list(map(str,command)),cwd=ROOT,stdout=f,stderr=subprocess.STDOUT)
  result.append({'check':name,'exit_code':run.returncode,'seconds':time.monotonic()-start,'log':str(log)})
  (args.output/'checks.json').write_text(json.dumps(result,indent=2)+'\n');print(name,'PASS'if run.returncode==0 else'FAIL',flush=True)
  if run.returncode:raise SystemExit(run.returncode)
if __name__=='__main__':main()
