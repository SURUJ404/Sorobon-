# soroban-sim-host-p30

An isolated copy of the `soroban-env-host` **30.0.0** source for Soroban Sim's
protocol-30 transaction application path. It provides a distinct Cargo package
identity, so the simulation host's `recording_mode` feature is not unified into
the host that applies transactions. That feature changes CPU metering on writes.

`src/` and `build.rs` are unchanged upstream source. `UPSTREAM.json` records the
upstream version, commit, published archive checksum and each copied file's
SHA-256. The package manifest and this README are Soroban Sim's packaging
changes; `UPSTREAM-README.md` preserves the upstream overview. Apache-2.0
licensing and Stellar Development Foundation attribution are retained.

This is an implementation dependency of `soroban-sim-core-p30`, not a general
replacement for Stellar's host crate. Do not enable `recording_mode` or
`testutils` in an exact-apply consumer. The CLI's network replay tests exercise
metering with both host packages linked together. Upstream's test/benchmark
targets are not run from this compatibility package.

See [release maintenance](https://github.com/SURUJ404/Sorobon-/blob/main/docs/releases.md)
for the upgrade procedure.
