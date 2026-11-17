#!/usr/bin/env python3
"""Verify the intentionally unchanged upstream host copies against their byte manifests.

This checks provenance/drift, not host behavior. The CLI's offline replay suite
is the independent behavioral check for exact application metering.
"""
import hashlib
import json
from pathlib import Path

crates = Path(__file__).resolve().parents[1] / "crates"
for name in ("soroban-sim-host", "soroban-sim-host-p30"):
    root = crates / name
    upstream = json.loads((root / "UPSTREAM.json").read_text())
    actual = {
        path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in [root / "build.rs", *sorted((root / "src").rglob("*"))]
        if path.is_file()
    }
    expected = {path.replace("\\", "/"): digest for path, digest in upstream["files"].items()}
    if actual != expected:
        raise SystemExit(f"{name} differs from its upstream byte manifest; see docs/releases.md")
    print(f"Verified {len(actual)} unchanged upstream files from {upstream['crate']} {upstream['version']}")
