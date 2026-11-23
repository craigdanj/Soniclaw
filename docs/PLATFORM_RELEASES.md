# Platform validation and release tooling

Native releases now have a complete package manifest and a repeatable audit
that checks the staged files without executing or installing them. A dedicated
Windows/macOS/Linux workflow builds native exports and retains packages and
structured evidence. A workflow file does not assert that a remote run passed.

## Stage, audit, review

Build the native format first using the pinned SDK script. Collect notices and
stage a new developer package:

```sh
python3 tools/collect-licenses.py --output build/release-licenses --format-deps vendor/formats
python3 tools/package-release.py build/formats/Release/SoniclawReference.vst3 --output build/release-stage --version 0.31.0 --licenses build/release-licenses
python3 tools/release-gate.py build/release-stage/SoniclawReference-0.31.0-linux-x86_64 --platform linux --architecture x86_64 --report build/release-audit.json
```

Use the actual platform/architecture and new staging directory. Supported audit
platform names are `linux`, `windows`, `macos`; architectures `x86_64`, `aarch64`
and macOS `universal2`. `package-release.py --helper PATH` optionally includes an
explicit matching isolation helper in a developer bundle. Applications shipping
isolation need the matching host/helper backend; exported reference plug-ins do
not themselves require a helper. This option stages a file, not an installer or
an isolation-enabled application.

Schema2 `manifest.json` hashes **every staged regular payload and notice file**,
with normalized relative paths, product/version/platform, signed-claim flag and
optional helper path. The audit rejects missing/extra/tampered files, unsafe or
escaped paths, missing framework notices, unrecognized native payloads, platform
mismatches and unsupported/mismatched native architectures. It reads ELF64,
PE and thin/universal64-bit Mach-O headers. These structural checks do not prove
that a binary links, satisfies an audio ABI, is free of malware, or works in a
DAW. Run the existing ABI/native suites too.

The report has separate `automated_status` and `acceptance_status`. Exit0 means
requested automated audit passed; physical/DAW acceptance can remain pending.
Exit1 means audit/acceptance validation failed. `--require-acceptance` returns2
when audited bytes pass but operator acceptance is still pending. It never
turns an unexecuted check into a pass. This is an opt-in release policy flag,
not a requirement to publish or a user permission step.

## Target-machine evidence

The existing compatibility suite supplies the manual case template and
`validate_acceptance`: DAW scan/state/automation/bypass/latency, editor HiDPI,
audio loopback, device unplug and disk-error behavior. For release auditing,
add `release_manifest_sha256` with the exact manifest hash from the audit report.
Completed attestations need platform, tester, timezone-qualified test timestamp,
OS/environment and per-case evidence. Run:

```sh
python3 tools/release-gate.py STAGE --platform linux --architecture x86_64 --acceptance acceptance.json --require-acceptance --report audited-acceptance.json
```

The audit refuses an attestation for another platform or manifest. Pending and
skipped cases keep acceptance pending. Evidence is an operator attestation,
identified as such; the tool does not independently execute hardware tests or
verify the truth of free-form notes. A changed payload produces a new manifest
and needs fresh matching acceptance. Preserve reports next to the published
artifact in your own release process.

`signature_verified` stays false in this structural audit. A staged macOS
codesign request still uses the package tool's existing explicit identity
option and platform verification. Notarization, Windows Authenticode credentials,
certificate management and store publishing are separate operations. Nothing
here signs, uploads, installs or publishes a release automatically.

## Workflow and ownership

`.github/workflows/release-validation.yml` runs on manual dispatch or version
tags. It uses read-only repository permissions, existing source compatibility
checks, native reference export builds from pinned SDKs, native VST3 audio/state/
MIDI acceptance, and Linux-only independent musical/fault-isolation fixtures.
It collects supplied notices, stages and audits a VST3 package per native
runner, and retains evidence/artifacts even on failure. macOS additionally
builds AUv2; broad AU/DAW/editor/hardware acceptance remains pending until run.

All tools run off audio threads. Audits stream file digests and inspect bounded
native headers (up to1MiB/header); inventories are limited to20000 entries.
Commands return errors and retain their report for diagnosis. Use a new stage
for a retry; keep known-good artifacts untouched. Signing/deployment secrets
are not read by the new workflow. Source ZIP generation and per-minor release
notes continue through `minor-release.py`.

## Compatibility and actual validation

`package-release.py` now emits schema2. Historical schema1/unspecified manifests
remain usable as historical packages, but the new gate requires rebuilding a
complete schema2 inventory. `release-gate.py` is a Python3.11+ CLI; existing
source, plug-in, session and project schemas do not change.

Run `python3 -m unittest discover -s tools/tests`. Tests cover native machine
mismatch, header truncation, byte tampering, extra files, escaped paths and
manifest-specific acceptance. The release report records actual local native
staging/auditing and expected pending-acceptance behavior. Remote CI runs,
installer installation, signing/notarization, physical devices and external
DAWs are not claimed without recorded execution.
