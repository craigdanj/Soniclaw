#!/usr/bin/env python3
"""Run an isolated software-only PipeWire graph; never changes the desktop server."""
import argparse,os,subprocess,tempfile,time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--server',default='pipewire');p.add_argument('--probe',default='target/debug/soniclaw-pipewire-probe');p.add_argument('--report',required=True);args=p.parse_args()
root=Path(__file__).resolve().parents[1]
report=Path(args.report).resolve()
if report.exists():raise SystemExit('report must be new')
with tempfile.TemporaryDirectory(prefix='soniclaw-pipewire-')as directory:
    env=os.environ.copy();env['XDG_RUNTIME_DIR']=directory;env['PIPEWIRE_REMOTE']='soniclaw-test'
    with report.with_suffix('.daemon.log').open('w')as log:
        daemon=subprocess.Popen([args.server,'-c',str(root/'examples/pipewire-probe/headless.conf')],env=env,stdout=log,stderr=log)
        try:
            deadline=time.monotonic()+5
            while not Path(directory,'soniclaw-test').exists():
                if daemon.poll()is not None:raise RuntimeError(f'daemon failed: inspect {log.name}')
                if time.monotonic()>deadline:raise TimeoutError('private PipeWire socket not created')
                time.sleep(.02)
            subprocess.run([args.probe,'soniclaw-test',str(report)],env=env,timeout=15,check=True)
        finally:
            daemon.terminate()
            try:daemon.wait(timeout=3)
            except subprocess.TimeoutExpired:daemon.kill();daemon.wait()
print(report)
