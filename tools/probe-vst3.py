#!/usr/bin/env python3
"""Supervise native VST3 factory inspection. Contains scan crashes/hangs, not hostile code."""
import argparse,json,pathlib,subprocess,tempfile,time

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('bundle',type=pathlib.Path);p.add_argument('output',type=pathlib.Path);p.add_argument('--sdk',type=pathlib.Path,default=pathlib.Path('target/debug/soniclaw-sdk-tools'));p.add_argument('--timeout',type=float,default=10.);a=p.parse_args()
 if not 0.1<=a.timeout<=120:p.error('timeout must be 0.1..120 seconds')
 if a.output.exists():p.error('choose a new output file')
 started=time.monotonic();report={'bundle':str(a.bundle.resolve()),'timeout_seconds':a.timeout,'is_security_sandbox':False}
 with tempfile.TemporaryDirectory(prefix='soniclaw-vst3-probe-')as tmp:
  dest=pathlib.Path(tmp)/'native.json';cmd=[str(a.sdk.resolve()),'--vst3-probe',str(a.bundle.resolve()),str(dest)]
  try:
   proc=subprocess.run(cmd,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=a.timeout,check=False);report.update(exit_code=proc.returncode,stdout=proc.stdout[-65536:].decode(errors='replace'),stderr=proc.stderr[-65536:].decode(errors='replace'))
   report['status']='passed'if proc.returncode==0 and dest.exists()else'failed'
   if report['status']=='passed':report['metadata']=json.loads(dest.read_text())
  except subprocess.TimeoutExpired:report.update(status='timed_out')
  except OSError as error:report.update(status='failed',error=str(error))
 report['seconds']=round(time.monotonic()-started,3);a.output.write_text(json.dumps(report,indent=2)+'\n');print(report['status']);return 0 if report['status']=='passed'else 1
if __name__=='__main__':raise SystemExit(main())
