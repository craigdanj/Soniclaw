# Plug-in crash isolation

This release adds an opt-in VST3 child-process owner with explicit checkpoint
recovery. Module loading and native DSP run in the selected helper. A crash or
watchdog timeout revokes playback while the application remains alive. This is
process fault containment; it is not an OS permission/security sandbox.

## Build and workflow

Enable `soniclaw`'s `plugin-isolation` feature, or depend on `soniclaw-vst3` with
`features=["isolation"]`. Build and ship the matching helper next to your app or
at an explicit installed path:

```sh
cargo build -p soniclaw-isolation-helper --locked
```

The workspace helper target builds the retained MIT upstream source against the
same patched backend and lockfile as the host. Deploy compatible host/helper
builds together. No system installation is performed by the build.

1. Select a trusted helper executable, native module and class on the main
   thread. Use `isolation::Config {helper,response_timeout_ms,max_recoveries}`.
2. Call unsafe `Isolated::load`, inspect parameters and restore a checkpoint or
   set controls while stopped. `TrustedLoader::load_isolated_vst3` additionally
   enforces the module allowlist; helper trust remains an explicit contract.
3. Call `checkpoint()` before playback. It snapshots opaque native state and
   writable normalized parameters. Then `start_buffered()` returns the ordinary
   fixed-memory `Processor` to move to the device callback.
4. Poll the main owner. On failure, `poll()` returns an error and `status()`
   reports `Faulted`. Keep device callbacks running; they return silence.
5. Invoke `recover()` explicitly from the control thread. It revokes and drops
   the old owner, starts a fresh helper and restores the checkpoint plus controls
   subsequently set through this owner. It returns `Ready`; obtain and connect a
   new processor. The retained old processor stays silent permanently.

```rust,no_run
use soniclaw_vst3::isolation::{Config,Isolated};
use soniclaw_core::ProcessSpec;
let config=Config {helper:std::path::PathBuf::from("/opt/my-app/vst3-host-helper"),
    response_timeout_ms:500,max_recoveries:3};
let spec=ProcessSpec {sample_rate:48000.,max_block_frames:256,channels:2};
// SAFETY: application explicitly selects the trusted helper/module/dependencies.
let mut owner=unsafe {Isolated::load(std::path::Path::new("Trusted.vst3"),None,spec,config)}?;
let saved=owner.checkpoint()?;
let audio=owner.start_buffered()?;
// Move audio into the device callback; owner.poll() stays on main.
// If poll fails: owner.recover()?; reconnect owner.start_buffered()? afterwards.
owner.stop()?;
# let _=(saved,audio);
# Ok::<(),Box<dyn std::error::Error+Send+Sync>>(())
```

## Faults, deadlines and recovery budget

`Phase` distinguishes `Ready`, `Running`, `Stopped`, `Faulted` and `Quarantined`.
Status includes the current helper PID, last control-side error, recovery
attempt count and bridge counters. Poll these on the control side. Ordinary
process IPC has a configurable50–10000ms response deadline; timeout kills the
helper. Native load/state/other slow command classes retain the backend's larger
startup deadline (at least30seconds). A device callback never waits for either:
all IPC/native locking is confined to the worker/control threads. Main-thread
poll/recovery can block up to those control deadlines. Closing a faulty helper
may also wait for its bounded teardown.

A native process fault silences its packet and revokes that run. Before a hang
reaches the watchdog deadline, output can underrun and input queues can fill;
those failures are counted and audio is silent where unavailable. Do not read
those expected fault-injection counters as successful glitch-free processing.
An entire live graph should gate its failed node according to application policy.
The existing1024-frame bridge latency remains; JSON/pipe IPC adds worker cost.
This release does not claim direct-path performance or shared-memory IPC.

Automatic respawn/retry is disabled. Every explicit recovery attempt consumes
budget, including failed loads/restores. Budget1–8 is per owner lifetime. After
exhaustion, recovery returns an error, revokes any current run and marks the
owner quarantined. The application may persist its own product exclusion list;
there is no hidden permanent blacklist. Reconstructing a new owner explicitly
starts a new budget.

## State, ownership and bounds

`Checkpoint` schema1 contains the existing digest-checked VST3 `State` and at
most4096 writable normalized parameter values. `from_json` bounds input to24MiB
and validates schema, identity, state size/digest and finite0...1 values.
`to_json` validates before serialization. Write/read the JSON off the callback
through the application's transactional project storage. Saved paths never load
code by themselves: explicit trusted `Isolated::load` selects the executable
code, and restore verifies the class identity.

Snapshot/restore require stopped processing. Held notes, pending events,
audio buffers and DSP history are not restored. Controls set through this owner
after the checkpoint are replayed; arbitrary editor/controller changes are not
automatically captured. Refresh the checkpoint while stopped to include them.
If native state is unsupported, skip checkpointing and explicitly reapply your
known parameter values; recovery otherwise starts from native defaults.

The owner remains main-thread-only. Callback packets/queues keep the existing
bounded stereo bridge limits; owner drop revokes and joins before teardown.
Helper and plug-in filesystem/dependency trust must remain valid for the owner
lifetime, including recovery. This release exposes stereo buffered VST3
isolation. CLAP, direct mode, multibus isolation, editor hosting and v0.27's full
musical-context API are not added to this owner. Those in-process APIs retain
their existing documented contracts. Workbench's existing host panel remains
in-process; custom desktop apps can use this owner as in the runnable example.

## Reproduce native acceptance

On Linux build the opt-in independent fault fixture and run the acceptance app:

```sh
python3 tools/build-formats.py --fetch --no-editor --protocol-fixture
cargo build -p soniclaw-isolation-helper -p soniclaw-crash-host --locked
ulimit -c 0
target/debug/soniclaw-crash-host target/debug/vst3-host-helper build/formats/Release/SoniclawReference.vst3 build/formats/fixtures/HostProtocolFixture.vst3 new-crash-report
```

The last path must be a new directory. This deliberately kills only its own
helper, injects `abort()` and a2second DSP stall into the dedicated fixture, and
checks application survival, callback silence/allocation counts, recovery to a
new PID, restored gain and old-callback revocation. Recovery-budget exhaustion
is also checked. `checkpoint.json` and `report.json` record results. Fault
injection is compiled only into the optional independent fixture, not reference
products. Unit tests reject invalid policy without native execution. Exact
executed Linux evidence is in the release report; external DAW/hardware and
Windows/macOS runtime checks remain pending.
