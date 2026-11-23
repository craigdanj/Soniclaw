#!/usr/bin/env python3
"""Repeatable source, exported ABI and target-machine acceptance reports (Python 3.11+)."""
from __future__ import annotations
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time
import tomllib
import zipfile
ROOT = Path(__file__).resolve().parents[1]
MANUAL_CASES = ("daw-scan", "daw-state-reopen", "daw-automation", "daw-bypass-latency", "editor-hidpi", "audio-loopback", "device-unplug", "recording-disk-error")


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def source_contract(root):
    """Check active package versions and release links, excluding archived baselines."""
    root = Path(root)
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    version = manifest["workspace"]["package"]["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("workspace version must be a release semver")
    paths = [root / "Cargo.toml"]
    for pattern in ("crates/*/Cargo.toml", "examples/*/Cargo.toml", "tools/*/Cargo.toml"):
        paths += sorted(root.glob(pattern))
    packages = set()
    for path in paths:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
        package = data.get("package")
        if package:
            packages.add(package["name"])
            v = package.get("version")
            if v != version and v != {"workspace": True}:
                raise ValueError(f"stale package version: {path.relative_to(root)}")
        tables = [data.get("dependencies", {}), data.get("dev-dependencies", {}), data.get("build-dependencies", {}), data.get("workspace", {}).get("dependencies", {})]
        for target in data.get("target", {}).values():
            tables += [target.get("dependencies", {}), target.get("dev-dependencies", {}), target.get("build-dependencies", {})]
        for table in tables:
            for name, dep in table.items():
                if isinstance(dep, dict) and "path" in dep and name.startswith("soniclaw") and "version" in dep and dep["version"] != version:
                    raise ValueError(f"stale internal dependency: {path.relative_to(root)}: {name}")
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    found = {p["name"] for p in lock["package"] if p["name"] in packages and p["version"] == version}
    if found != packages:
        raise ValueError(f"missing/current lock packages: {sorted(packages-found)}")
    major, minor, _ = map(int, version.split("."))
    # Check each pre-1.0 milestone in this project's established series.
    releases = [f"0.{m}.0" for m in range(1, minor + 1)] if major == 0 else [version]
    for v in releases:
        notes = root / f"docs/releases/v{v}.md"
        if not notes.is_file() or v not in (root / "docs/releases/README.md").read_text(encoding="utf-8"):
            raise ValueError(f"missing release notes/index: {v}")
    if version not in (root / "README.md").read_text(encoding="utf-8") or version not in (root / "CHANGELOG.md").read_text(encoding="utf-8"):
        raise ValueError("README/changelog missing current release")
    # Validate the maintained entry points and current release guide links only.
    checked = set()
    todo = [root / "README.md", root / "docs/releases/README.md", root / f"docs/releases/v{version}.md"]
    while todo:
        path = todo.pop()
        if path in checked:
            continue
        checked.add(path)
        for link in re.findall(r"\[[^\]]*\]\(([^)]+)\)", path.read_text(encoding="utf-8")):
            if "://" in link or link.startswith(("#", "mailto:")):
                continue
            relative = link.split("#", 1)[0].split(" ", 1)[0]
            if not relative:
                continue
            target = (path.parent / relative).resolve()
            # Validation reports are created by the release sealing step.
            if "/docs/validation/" in str(target) and not target.exists():
                continue
            if not target.exists():
                raise ValueError(f"broken link: {path.relative_to(root)} -> {relative}")
            if path.name == f"v{version}.md" and target.suffix == ".md" and target.parent == root / "docs":
                todo.append(target)
    return {"version": version, "packages": len(packages), "manifests": len(paths), "release_notes": len(releases), "entry_points": len(checked)}


def verify_archive(path):
    """Stream every manifest member; reject traversal/duplicates/unlisted source files."""
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError("archive contains duplicate names")
        for name in names:
            parts = Path(name).parts
            if name.startswith(("/", "\\")) or ".." in parts or "\\" in name:
                raise ValueError("unsafe archive member")
        candidates = [n for n in names if n.endswith("/SOURCE_MANIFEST.json")]
        if len(candidates) != 1:
            raise ValueError("archive must contain one source manifest")
        manifest_name = candidates[0]
        prefix = manifest_name.removesuffix("SOURCE_MANIFEST.json")
        if archive.getinfo(manifest_name).file_size>8*1024*1024 or len(names)>100_000:
            raise ValueError("archive manifest/member limit exceeded")
        manifest = json.loads(archive.read(manifest_name))
        records = manifest["files_sha256"]
        # Source release manifests map relative paths to SHA256 values.
        if isinstance(records, list):
            records = {r["path"]: r["sha256"] for r in records}
        expected = {prefix + p for p in records} | {manifest_name}
        actual = {n for n in names if not n.endswith("/")}
        if expected != actual:
            raise ValueError(f"archive member mismatch: missing {len(expected-actual)}, extra {len(actual-expected)}")
        for relative, record in records.items():
            expected_hash = record["sha256"] if isinstance(record, dict) else record
            h = hashlib.sha256()
            with archive.open(prefix + relative) as f:
                for block in iter(lambda: f.read(1024 * 1024), b""):
                    h.update(block)
            if h.hexdigest() != expected_hash:
                raise ValueError(f"archive hash mismatch: {relative}")
        return {"version": manifest.get("version"), "files": len(records), "sha256": digest(Path(path))}


def acceptance_template():
    return {"schema": 1, "platform": "", "tester": "", "tested_at": "", "environment": {"os": "", "daw": "", "device": "", "driver": "", "buffer_frames": None}, "cases": [{"id": case, "status": "not_run", "evidence": "", "reason": "Target-machine check pending"} for case in MANUAL_CASES]}


def validate_acceptance(document):
    if document.get("schema") != 1 or document.get("platform") not in ("linux", "macos", "windows"):
        raise ValueError("acceptance needs schema=1 and platform linux/macos/windows")
    cases = document.get("cases", [])
    if not isinstance(cases, list) or len(cases) != len(MANUAL_CASES) or {c.get("id") for c in cases} != set(MANUAL_CASES):
        raise ValueError("acceptance cases must match the template, with no duplicates")
    for case in cases:
        status = case.get("status")
        if status not in ("passed", "failed", "skipped", "not_run"):
            raise ValueError("invalid acceptance case status")
        if status in ("passed", "failed"):
            if not case.get("evidence") or not document.get("tester") or not document.get("tested_at") or not document.get("environment", {}).get("os"):
                raise ValueError("attested results need tester, date, OS and evidence")
        elif not case.get("reason"):
            raise ValueError("unrun/skipped cases need a reason")
    status = "failed" if any(c["status"] == "failed" for c in cases) else "passed" if all(c["status"] == "passed" for c in cases) else "pending"
    return {"platform": document["platform"], "status": status, "source": "operator_attestation", "cases": cases}


class Runner:
    def __init__(self, output, root=ROOT, timeout=300):
        self.output = Path(output)
        self.root = Path(root)
        self.timeout = timeout
        self.checks = []

    def record(self, name, status, **details):
        check = {"name": name, "status": status, **details}
        self.checks.append(check)
        self.save()
        print(f"{status.upper():7} {name}", flush=True)
        return check

    def save(self):
        report = {"schema": 1, "environment": {"system": platform.platform(), "machine": platform.machine(), "python": platform.python_version()}, "checks": self.checks,
                  "automated_status": "failed" if any(c["status"] == "failed" for c in self.checks) else "passed",
                  "acceptance_status": "pending", "scope": "Local automated checks; physical audio, real DAW and other operating systems require explicit acceptance records."}
        (self.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")

    def function(self, name, function):
        try:
            return self.record(name, "passed", details=function())
        except Exception as error:
            return self.record(name, "failed", error=str(error))

    def command(self, name, command, timeout=None):
        start = time.monotonic()
        log = self.output / (name + ".log")
        code = None
        error = None
        try:
            with log.open("w") as stream:
                env = os.environ.copy()
                if name == "docs":
                    env["RUSTDOCFLAGS"] = "-D warnings"
                result = subprocess.run(list(map(str, command)), cwd=self.root, stdout=stream, stderr=subprocess.STDOUT, timeout=timeout or self.timeout, env=env)
                code = result.returncode
        except subprocess.TimeoutExpired:
            error = "child process timed out and was killed"
        except OSError as e:
            error = str(e)
        return self.record(name, "passed" if code == 0 else "failed", command=list(map(str, command)), exit_code=code, error=error, seconds=round(time.monotonic()-start, 3), log=log.name)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new report directory")
    parser.add_argument("--full", action="store_true", help="build, lint, test, docs, exports and workflows")
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--probe", type=Path, help="explicit trusted native CLAP library, isolated child probe")
    parser.add_argument("--archive", type=Path, help="verify a source release ZIP")
    parser.add_argument("--acceptance", type=Path, action="append", default=[])
    args = parser.parse_args(argv)
    if not 0 < args.timeout <= 3600:
        parser.error("timeout must be 0..3600 seconds")
    args.output.mkdir(parents=True, exist_ok=False)
    runner = Runner(args.output.resolve(), timeout=args.timeout)
    runner.function("source-contract", lambda: source_contract(ROOT))
    runner.command("rust-toolchain", ["rustc", "-Vv"], timeout=10)
    runner.command("python-tool-tests", [sys.executable, "-m", "unittest", "discover", "-s", "tools/tests", "-v"])
    suffix = ".dll" if sys.platform == "win32" else ".dylib" if sys.platform == "darwin" else ".so"
    prefix = "" if sys.platform == "win32" else "lib"
    binary = ROOT / "target/debug" / ("soniclaw-sdk-tools" + (".exe" if sys.platform == "win32" else ""))
    library = lambda name: ROOT / "target/debug" / (prefix + "soniclaw_" + name + suffix)
    cargo_flags = ["--locked"] + (["--offline"] if args.offline else [])
    if args.full:
        commands = [("fmt", ["cargo", "fmt", "--all", "--check"]),
                    ("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--all-features", *cargo_flags, "--", "-D", "warnings"]),
                    ("tests", ["cargo", "test", "--workspace", "--all-features", *cargo_flags]),
                    ("docs", ["cargo", "doc", "--workspace", "--all-features", "--no-deps", *cargo_flags]),
                    ("build", ["cargo", "build", "-p", "soniclaw-sdk-tools", "-p", "soniclaw-host-fixture", "-p", "soniclaw-workbench", "-p", "soniclaw-clap-smoke", *cargo_flags])]
        for name, command in commands:
            runner.command(name, command)
        if all(c["status"] == "passed" for c in runner.checks if c["name"] in {"build"}):
            smoke = ROOT / "target/debug" / ("soniclaw-clap-smoke" + (".exe" if sys.platform == "win32" else ""))
            for i in range(3):
                runner.command(f"export-reopen-{i}", [smoke, library("workbench"), "--workbench"])
            for mode in ("catalog", "sampler", "automation"):
                runner.command(mode + "-workflow", [binary, f"--{mode}-demo", runner.output / (mode + "-demo")])
            runner.command("host-contract", [binary, "--host-contract", library("host_fixture"), runner.output / "host-contract.json"])
            runner.command("live-rack-contract", [binary, "--live-rack-demo", library("host_fixture"), runner.output / "live-rack-demo"])
            runner.command("processor-audit", [binary, "--audit", runner.output / "processor-audit.json"])
    else:
        runner.record("build-export-workflows", "skipped", reason="Use --full for builds, Rust checks, exported ABI reopen tests and workflows")
    if args.probe:
        runner.command("isolated-native-probe", [binary, "--probe-trusted", args.probe.resolve(), runner.output / "probe.json"])
    if args.archive:
        runner.function("archive-manifest", lambda: verify_archive(args.archive))
    acceptances = []
    for i, path in enumerate(args.acceptance):
        check = runner.function(f"target-acceptance-{i}", lambda p=path: validate_acceptance(json.loads(p.read_text(encoding="utf-8"))))
        if check["status"] == "passed":
            acceptances.append(check["details"])
    for target in ("linux", "macos", "windows"):
        if not any(a["platform"] == target for a in acceptances):
            runner.record("acceptance-" + target, "skipped", reason="No target-machine operator acceptance record supplied")
    template = acceptance_template()
    (runner.output / "acceptance-template.json").write_text(json.dumps(template, indent=2) + "\n")
    report = json.loads((runner.output / "report.json").read_text(encoding="utf-8"))
    report["acceptances"] = acceptances
    report["acceptance_status"] = "failed" if any(a["status"] == "failed" for a in acceptances) else "passed" if all(any(a["platform"] == target and a["status"] == "passed" for a in acceptances) for target in ("linux", "macos", "windows")) else "pending"
    report["generated_at_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    (runner.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    return 1 if report["automated_status"] == "failed" or report["acceptance_status"] == "failed" else 0


if __name__ == "__main__":
    sys.exit(main())
