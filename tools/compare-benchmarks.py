#!/usr/bin/env python3
"""Summarize median batch CPU and optionally gate a pinned-runner regression."""
import argparse,csv,json,pathlib,statistics
NAMES={0:'Pass-through',1:'Static gain',2:'Low-pass + gain',3:'Automated gain',4:'Automated cutoff'}
def load(path):
 groups={}
 with path.open() as f:
  for row in csv.DictReader(f):
   key=(row['plugin'],int(row['mode']),int(row['frames']))
   groups.setdefault(key,[]).append(float(row['cpu_ns_per_callback']))
 return {k:{'median_ns':statistics.median(v),'min_ns':min(v),'max_ns':max(v),'rounds':len(v)} for k,v in groups.items()}
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('baseline',type=pathlib.Path);p.add_argument('current',type=pathlib.Path);p.add_argument('--output',type=pathlib.Path,default=pathlib.Path('benchmarks/results'));p.add_argument('--fail-regression-percent',type=float,help='Use only on a controlled, repeatable machine');args=p.parse_args();old,new=load(args.baseline),load(args.current);args.output.mkdir(parents=True,exist_ok=True)
 rows=[]
 for key in sorted(old.keys()&new.keys()):
  a,b=old[key],new[key];rows.append({'plugin':key[0],'mode':key[1],'workload':NAMES[key[1]],'frames':key[2],'baseline_us':a['median_ns']/1000,'current_us':b['median_ns']/1000,'speedup':a['median_ns']/b['median_ns'],'baseline_min_us':a['min_ns']/1000,'baseline_max_us':a['max_ns']/1000,'current_min_us':b['min_ns']/1000,'current_max_us':b['max_ns']/1000})
 if not rows:raise SystemExit('No matching benchmark cells')
 with (args.output/'comparison.csv').open('w',newline='')as f:w=csv.DictWriter(f,fieldnames=rows[0]);w.writeheader();w.writerows(rows)
 text=['# Soniclaw v0.2 vs v0.3 Linux processing','', 'Median batch-average thread CPU per callback, microseconds; 48 kHz stereo. Lower is better. Nine shuffled rounds per cell. Baseline crate sources are frozen and hashed.','', '| Workload | Frames | v0.2 µs | v0.3 µs | Speedup |','|---|---:|---:|---:|---:|']
 for r in rows:text.append(f"| {r['workload']} | {r['frames']} | {r['baseline_us']:.3f} | {r['current_us']:.3f} | {r['speedup']:.2f}× |")
 text+=['','Both versions are measured with the same independent exported-CLAP host, compiler version and O3/thin-LTO release settings. The baseline library does not include a GUI. The v0.3 reference does, but its window is closed during timing. Both use the same processing modes and parameter values.','', 'Shared VM: no exclusive physical core or fixed CPU frequency. Runs are sequential, not interleaved between versions. Do not interpret small gaps as significant. No JUCE rerun or language-wide conclusion is implied. Timing includes common host/event bookkeeping. Raw batches retain round ranges. These are CPU costs, not physical audio latency.']
 (args.output/'PERFORMANCE.md').write_text('\n'.join(text)+'\n');(args.output/'comparison.json').write_text(json.dumps(rows,indent=2)+'\n')
 if args.fail_regression_percent is not None:
  failed=[r for r in rows if r['current_us']>r['baseline_us']*(1+args.fail_regression_percent/100)]
  if failed:raise SystemExit(f'{len(failed)} cells exceeded the configured regression threshold')
 print(args.output/'PERFORMANCE.md')
if __name__=='__main__':main()
