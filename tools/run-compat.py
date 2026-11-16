#!/usr/bin/env python3
"""Build/run the independent Linux exported-ABI host and allocation probes."""
import argparse,json,os,pathlib,platform,shutil,subprocess,sys
ROOT=pathlib.Path(__file__).resolve().parents[1]
def run(args,**kwargs):subprocess.run(list(map(str,args)),check=True,**kwargs)
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--library',type=pathlib.Path,required=True);p.add_argument('--clap-sdk',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,default=ROOT/'benchmarks/results/current');p.add_argument('--gui',action='store_true');args=p.parse_args()
 if sys.platform!='linux':p.error('this glibc interposer/CPU host currently targets Linux; Rust behavior tests are portable')
 if args.gui and not os.environ.get('DISPLAY'):p.error('--gui needs an X11 display (including Xvfb)')
 build=ROOT/'build/compat';build.mkdir(parents=True,exist_ok=True);output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
 run(['cc','-O2','-fPIC','-shared',ROOT/'tools/compat/alloc-hook.c','-o',build/'alloc-hook.so'])
 run(['c++','-O3','-std=c++17',f'-I{args.clap_sdk.resolve()/"include"}',ROOT/'tools/compat/host.cpp','-ldl','-lpthread','-lX11','-o',build/'host'])
 env=os.environ.copy();env['LD_PRELOAD']='./alloc-hook.so'+((':'+env['LD_PRELOAD']) if env.get('LD_PRELOAD') else '')
 command=[build/'host',args.library.resolve(),output]
 if args.gui:command+=['--gui']
 run(command,env=env,cwd=build)
 (output/'environment.json').write_text(json.dumps({'system':platform.platform(),'processor':platform.processor(),'cpuinfo':pathlib.Path('/proc/cpuinfo').read_text(),'rustc':subprocess.check_output([shutil.which('rustc') or pathlib.Path.home()/'.cargo/bin/rustc','-Vv'],text=True),'cxx':subprocess.check_output(['c++','--version'],text=True),'gui_requested':args.gui,'hardware_audio_tested':False},indent=2)+'\n')
if __name__=='__main__':main()
