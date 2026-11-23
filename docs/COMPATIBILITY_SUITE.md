# Compatibility and release verification suite (0.15)

`tools/compatibility-suite.py` produces structured, repeatable source, build, native ABI, workflow and acceptance evidence. Its reports distinguish automated checks from operator-reported target-machine acceptance. A Linux source test does not become a Windows, macOS, DAW or audio-hardware pass.

## Quick start

Requires Python 3.11+ (`tomllib`), the workspace Rust toolchain and the native build prerequisites described in README. The quick profile checks active package/dependency/lock versions, release documentation links and Python report-tool behavior. Full builds require the existing ALSA/windowing dependencies on Linux and platform equivalents elsewhere.

```sh
# Source/document contract and Python tooling tests; creates a NEW directory.
python3 tools/compatibility-suite.py --output /tmp/new-source-check

# Full local check: Rust checks/builds, ABI reopen cycles, feature demos and audits.
python3 tools/compatibility-suite.py --full --offline \
  --output /tmp/new-full-check --timeout 900

# Verify a previously packaged source ZIP; checks every manifest hash and membership.
python3 tools/compatibility-suite.py --archive /path/soniclaw-source.zip \
  --output /tmp/new-archive-check

# Explicitly selected trusted native code; loaded only inside a child process.
python3 tools/compatibility-suite.py --probe /trusted/MyEffect.clap \
  --output /tmp/new-plugin-probe --timeout 30

python3 -m unittest discover -s tools/tests -v
```

The probe uses the already built `target/debug/soniclaw-sdk-tools` executable. Build it first, or combine `--probe` with `--full`. The tool does not discover arbitrary installed plug-ins or silently execute their code. `--probe` performs library init, product enumeration, instance init, metadata inspection and destruction; it does **not** activate audio or create editors. A success is metadata/lifecycle evidence only. Native code remains trusted code even when isolated in a child process: this is crash/timeout containment, not a filesystem/network security sandbox. Parent timeouts kill the immediate child; plug-ins that spawn independent processes are outside that containment.

## Profiles and commands

| Check | Quick | Full | Evidence |
|---|---|---|---|
| Active manifests/internal versions and lockfile | Yes | Yes | Package/manifest counts and version |
| Every pre-1.0 minor's notes/index | Yes | Yes | Notes count |
| Maintained entry-point/current-guide links | Yes | Yes | Local target existence |
| Python report/timeout/archive/acceptance tests | Yes | Yes | Test log |
| Rust formatting, Clippy, all-feature tests, rustdoc | Skipped | Yes | Separate logs and exit codes |
| Workbench/fixture/SDK/smoke builds | Skipped | Yes | Build log |
| Exported workbench ABI reopen | Skipped | Three child runs | Export state/parameter/layout/latency behavior |
| Catalog, sampler and automation demos | Skipped | Yes | JSON/WAV/project evidence |
| Independent host and live-rack contracts | Skipped | Yes | Fixture timing, events, thread and lifecycle evidence |
| DSP rate/block/allocation/state audit | Skipped | Yes | Per-product audit JSON |
| Source ZIP integrity | With --archive | With --archive | SHA256/member count |
| Native metadata probe | With --probe | With --probe | Per-product metadata JSON |
| Hardware/DAW/other OS acceptance | Supplied records only | Supplied records only | Operator attestation, explicitly labeled |

The full suite compiles debug artifacts and does not imply release performance. `--offline` adds Cargo offline mode and requires dependencies to be cached. `--timeout` applies per command (default 300 seconds, maximum 3600). Run all-feature docs with warnings denied. Command failures are recorded and later independent checks continue; runtime workflows are skipped if the required build fails. The output directory must not already exist so evidence from runs is not mixed. Fix the failure and use a new directory. Child command logs contain their stdout/stderr; inspect them when a report is failed. A missing binary, crash, nonzero exit or timeout is a failure, never a pass.

The runner uses the conventional workspace `target/debug` layout. A custom `CARGO_TARGET_DIR` or cross-compilation target directory is not currently supported; use an ordinary host build for this runner. No network access is needed after dependencies are cached, unless the selected plug-in itself attempts it. Avoid putting native probe paths in automated pull-request jobs from untrusted contributors.

## Report format and API

`report.json` has schema 1, environment metadata, command/function checks and two independent statuses:

- `automated_status`: passed if no executed automated/validation check failed. Skipped checks remain explicit and do not add coverage. The quick profile reports its skipped build/export workflows.
- `acceptance_status`: pending unless **all** required cases on Linux, macOS and Windows are supplied as passed operator records; failed if a supplied case failed. Passing automated checks with pending acceptance exits 0 so CI can test source without fabricating hardware coverage. Failed automated checks or failed acceptance exit 1.

Each command record includes name/status/command/exit code/error/seconds/log. Source/archive function checks include details or an error. Logs and feature outputs are retained under the report directory. `acceptance-template.json` is always emitted. The runner writes the report after each check, so a terminated run retains partial evidence; only a completed run has final acceptance records and generation time. Durations are wall time for orchestration, not DSP benchmarks.

The Python module's reusable functions are `source_contract(root)`, `verify_archive(path)`, `acceptance_template()` and `validate_acceptance(document)`. `Runner.function` captures a validation result/error; `Runner.command` runs an isolated child with a bounded timeout and records its outcome. Inputs/serialization belong on a control or build thread, never in an audio callback. These APIs allocate, access files and may launch processes. The CLI is the maintained user-facing entry point; Python API calls are intended for release tooling.

Source checks cover active `crates/*`, `examples/*`, `tools/*` package manifests, workspace internal path versions and Cargo.lock. Frozen historical benchmark sources are excluded. Existing package versions inherited from workspace remain valid. The established 0.x series requires a note/index entry for every minor up to the current version. Link checks cover README, release index, the current release notes and its directly linked guide. Missing current validation reports are permitted before the sealing step; the release must then create them. This is not a universal Markdown/link/web crawler.

ZIP verification reads `SOURCE_MANIFEST.json` (`files_sha256`), checks each byte stream, rejects duplicate, traversal or unlisted members, and records the complete archive SHA256. The manifest is limited to 8 MiB and archives to 100,000 members. Verification does not extract or execute contents. It verifies integrity against the included manifest, not authenticity/signatures; an attacker could replace both source and manifest. Keep the outer SHA256 through a trusted release channel if authenticity matters. Release ZIPs omit build output/dependencies and retain all historical release notes/evidence.

## Target-machine acceptance workflow

Copy the emitted template, set `platform` to linux, macos or windows, fill tester/tested_at/environment, and run these cases on that target:

| Case ID | Operator action and evidence |
|---|---|
| daw-scan | DAW scans the packaged plug-in, lists the expected products, instantiates without a crash. Record DAW/version, product and scan log. |
| daw-state-reopen | Save a session with edited parameters/assets, close/reopen, compare restored controls and audio. Record session and observations. |
| daw-automation | Record/play host automation including exact boundary changes; inspect audible/recorded result and parameter state. |
| daw-bypass-latency | Compare bypass behavior, tails, latency reporting and compensated timing in that DAW. |
| editor-hidpi | Open/close/reopen and resize at target scale factors; use keyboard/pointer controls; retain screenshots and scale settings. |
| audio-loopback | Loop output back to input, measure latency/dropouts at several buffers, record device/driver/clock and method. |
| device-unplug | With an unsaved take, unplug/replug input/output separately; inspect retry status, monitoring mute reset and recording report. |
| recording-disk-error | Use a controlled full/unwritable test volume, finalize, inspect errors/frame accounting and recovery. Never fill a production volume. |

Set each case status to `passed`, `failed`, `skipped` or `not_run`. Passed/failed requires a nonempty evidence reference, tester, date and OS. Unrun/skipped requires a reason. The validator checks structure and required fields; evidence references are operator attestations, not independently verified files. A template with blank platform is intentionally incomplete; set the platform even for a pending submission.

```sh
python3 tools/compatibility-suite.py --output /tmp/new-with-acceptance \
  --acceptance /path/linux-acceptance.json \
  --acceptance /path/windows-acceptance.json
```

Supply all eight case IDs exactly once in each record. Missing/duplicate IDs, unknown statuses or unsupported schema/platform fail validation. One platform's pass does not cover another. Re-run and retain a new record after an affected device/backend/format change. Do not reuse historical evidence as a new execution pass.

## Performance gate and regression fixes

The existing `tools/compare-benchmarks.py` now rejects NaN, infinite, zero or negative CPU values. When `--fail-regression-percent` is used, the threshold must be finite/nonnegative and baseline/current benchmark cells must match exactly; missing workloads cannot silently disappear from the gate. Descriptive comparisons without a gate may still compare their intersection. Keep raw round data and environment metadata.

```sh
python3 tools/compare-benchmarks.py baseline.csv current.csv \
  --output /tmp/benchmark-comparison --fail-regression-percent 10
```

Only enable timing thresholds on a controlled runner. No fresh JUCE benchmark, CPU improvement or physical latency result is claimed in this release. Existing independent Linux C++ host/allocation interposer remains available through `tools/run-compat.py`; its external CLAP SDK prerequisite and Linux-only scope remain unchanged.

The expanded device conformance test found and fixed output layout selection: when stereo's supported buffer range excludes the requested buffer but a larger hardware layout supports it, unified mode now chooses the smallest layout satisfying **both** rate/map and buffer constraints. DSP remains stereo and routes into that hardware layout. Previously it selected stereo first and then rejected a valid device request. Additional conformance fixes saturate automation sample positions at u64::MAX, clear the native draft recorder latch on transport stop, and clear previously observed CLAP output events before a requested parameter flush. Explicit unsupported preferences still fail normally. This has a regression test; no new device-preference schema or migration is required.

## CI, persistence and limits

The existing Linux/macOS/Windows CI matrix now invokes the structured full suite and uploads its evidence, while retaining existing app/template/export checks. Workflow files are configuration only until CI actually runs; this delivery has not triggered or observed remote runners. Reports/templates use schema 1; old release evidence remains unchanged. No Soniclaw project state migration is introduced in 0.15. Retain report folders with the source revision/ZIP, and keep native target acceptance records alongside them. A report may contain local paths and plug-in metadata; review before sharing.

See [release notes](releases/v0.15.0.md), [local results](validation/v015/REPORT.md) and [delivery sequence](DELIVERY_PLAN.md). Local Linux synthetic fixtures, source checks and native UI captures do not establish real DAW, physical-device, third-party editor or Windows/macOS runtime acceptance. Those remain pending until performed and recorded.
