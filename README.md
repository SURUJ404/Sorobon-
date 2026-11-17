# Soroban Sim

Cross-platform Soroban simulation for browsers, WASI Preview 2 components, and native Rust.

## Why rehearse on a fork

Simulation costs and behavior depend on a network's ledger state and
configuration. Soroban Sim runs against a local fork and reads that
configuration from the forked ledger, letting you rehearse transactions without
submitting them to the live network.

## Installation

The command-line tool is the `soroban-sim-cli` package; its executable is
`soroban-sim`. Version 0.1.0 is published on
[crates.io](https://crates.io/crates/soroban-sim-cli). Install from source:

```sh
cargo install soroban-sim-cli --locked   # build from source; needs Rust 1.89+
soroban-sim --version
```

The Rust libraries are `soroban-sim-core`, `soroban-sim-facade` and
`soroban-sim-component` (0.1.0). `soroban-sim-core-p30`, `soroban-sim-facade-p30`
and `soroban-sim-cli-p30` are generated copies of core, the facade and the CLI's
library against the protocol-30 host, so the one `soroban-sim` binary runs both
protocols. `soroban-sim-host` (29.0.2) and `soroban-sim-host-p30` (30.0.0) are isolated
upstream application hosts used for exact native metering; their upstream
sources are recorded in
[`soroban-sim-host/UPSTREAM.json`](crates/soroban-sim-host/UPSTREAM.json) and
[`soroban-sim-host-p30/UPSTREAM.json`](crates/soroban-sim-host-p30/UPSTREAM.json).
The TypeScript package
`soroban-sim` (`packages/soroban-sim`) is not published to npm yet.
See [Publishing and release maintenance](docs/releases.md) for how releases are
cut, verified and recovered.

## Architecture

Soroban Sim puts the WebAssembly Component Model around the simulator, not around each Soroban contract:

```text
Browser UI
  -> TypeScript API + Web Worker
  -> Jco-transpiled WASI Preview 2 component
  -> soroban-simulation / soroban-env-host
  -> soroban-wasmi
  -> normal wasm32v1-none Soroban contract bytes
```

Contracts remain ordinary Soroban core Wasm modules. The embedded Soroban host loads them through Wasmi.

## Requirements

- Rust stable with targets `wasm32-wasip2` and `wasm32v1-none`
- Node 24+
- pnpm 11+
- Wasmtime CLI 48+
- Playwright Chromium for browser proof

```sh
rustup target add wasm32-wasip2 wasm32v1-none
brew install wasmtime # macOS; or install Wasmtime another way
pnpm install
pnpm --filter soroban-sim-web-demo exec playwright install chromium
```

## Current limitations

- The component imports WASI Preview 2 CLI/io/clock/random shims because Rust `std` and upstream dependencies currently pull them in. It does not import WASI filesystem or sockets, and performs no network or persistent-filesystem access.
- RPC-equivalence authority is demonstrated for two captured native-SAC cases —
  a read-only `decimals()` and a write+auth+event `transfer()` — not yet across
  arbitrary custom-wasm contracts or state-archival/restore paths. See
  `docs/differential-testing.md` ("RPC equivalence (M3)") for scope.
- Resource and fee estimates default to the host's raw metered cost
  (`SimulationAdjustmentConfig::no_adjustments()`). Setting
  `apply-rpc-adjustments` (`AdjustmentSource::RpcDefault`) reproduces
  stellar-rpc's adjustment model exactly; the parity harness confirms adjusted
  instructions = raw + 50000 on every fixture. All fixtures are captured at
  protocol 29, the live-workbench host's protocol (native-SAC from Testnet,
  custom-wasm `local-add-i32` from a local quickstart node), and match RPC's
  instructions and resource fee exactly.
- The single-call simulation API records auth rather than enforcing it against
  a supplied auth tree. The network-session engine (below) enforces and
  verifies every envelope and auth signature.

## Network sessions

`soroban-sim-core::session` rehearses many signed transactions in order over a
fork of a real network, and the `soroban-sim` CLI (`crates/soroban-sim-cli`)
runs a plan file through it and writes a JSON and a Markdown report. Nothing is
ever submitted; throwaway keys stand in for every real signer.

```sh
cargo run -p soroban-sim-cli -- session run examples/session/perch-nido-bootstrap.yaml \
  --wasm-dir path/to/release-wasm --differential
```

See `docs/session.md` for the design, the plan format, the signer adapters
(classic, OZ `oz_rule_ids`/`addr_oz_v1`, CAP-71 delegates, secp256r1, WebAuthn)
and what the rehearsal does and does not model.

`soroban-sim serve` puts the same session behind a Stellar RPC facade, so stock
`stellar`, stellar-sdk and the scripts built on them rehearse against a fork
unchanged, with `fork_*` methods for signer substitution, balances, the clock,
snapshots and the report:

```sh
cargo run -p soroban-sim-cli -- serve --fork mainnet --report rehearsal
export STELLAR_RPC_URL=http://127.0.0.1:8000
export STELLAR_NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015"
STELLAR_SIGN_WITH_KEY=<throwaway> ./your-deploy-script.sh
```

See `docs/rpc-facade.md`.

Both read a fork either lazily at RPC's latest ledger (`--fork-mode head`, the
default) or pinned to one ledger with every read rolled back to it through the
later ledgers' close meta (`--fork-mode checkpoint [--fork-ledger N]`).

### Simulate one envelope, inspect a fork, export snapshots

`soroban-sim simulate` runs one envelope through the same
`simulateTransaction` path the facade serves, so the response has stellar-rpc's
shape (`transactionData`, `minResourceFee`, `results`, `events`,
`stateChanges`) without starting a server. The envelope may be a file, stdin
(`-`), a literal base64 `TransactionEnvelope`, or JSON with a `transaction`
field:

```sh
cargo run -p soroban-sim-cli -- simulate --fork testnet tx.xdr --json
cargo run -p soroban-sim-cli -- simulate --snapshot base.json tx.xdr
```

`soroban-sim fork` opens a fork and reports its ledger, protocol, base fee and
reserve, passphrase and network id; `--json` is machine-readable. It is the
quick check for what a checkpoint actually pinned.

`snapshot export` turns a live fork into a replayable file. With
`--transaction` it reads the envelope's own keys and the footprint its
simulation discovers (including the source account and, with
`--fork-mode checkpoint`, at the pinned ledger); with `--keys`/`--key` it reads
exactly those ledger keys. `snapshot import` validates, canonicalizes and merges
files, refusing two different states for one key or two different fork points:

```sh
soroban-sim snapshot export --fork testnet --transaction tx.xdr --out base.json
soroban-sim snapshot import a.json b.json --out merged.json
```

RPC cannot enumerate the whole ledger, so export records the keys it was asked
for (plus the network settings every offline session needs), not an entire
network.

`soroban-sim replay` checks the session's transaction layer against the
network: it records recent real mainnet and testnet Soroban transactions with
the exact state each applied against, replays them through the session, and
compares results, return values, events, after-images, fees and core's
resource metrics. The committed sample in `fixtures/replay` replays exactly in
CI; see `docs/replay.md`.

```sh
cargo run --release -p soroban-sim-cli -- replay fetch --network mainnet --min-ledgers 55 --out mainnet.json.gz
cargo run --release -p soroban-sim-cli -- replay run fixtures/replay/*.json.gz --report fidelity
```

The web demo runs the same session in the browser. The component exports it as
the WIT `network-session` resource, and its log and fork cache live in
IndexedDB, so a reload resumes it. A service worker serves it at a virtual RPC
URL, so stellar-sdk and the demo's wallet pre-submit flow submit into the fork
unchanged. The panel shows a step timeline with each step's fees, auth and
ledger changes, opens any step in the time-travel debugger, and ends with the
report. Signing defaults to throwaway keys, and a real wallet signing into a
session gets a replay warning. `soroban-sim serve` and the browser share one
request handler, `crates/soroban-sim-facade`. See `docs/browser-session.md`.

## Sign dialog

`packages/soroban-sign-dialog` (`soroban-sign-dialog`) is an
embeddable `<soroban-sign-dialog>` web component for wallets and dapps. It
renders the full tree of requested authority from a transaction's XDR alone
(about 17 KB brotli, any protocol) under a plain summary, and lazily loads a
preview engine that re-records the tree with soroban-sim's host. The user sees
one status (will succeed, will be refused or unknown), a warning for entries
the transaction never uses, and, behind Details, balance changes, events and a
smart account's own verdict. The engine ships one reactor per host protocol
(`crates/soroban-sim-preview`, p29 and p30, about 470 KB brotli each) and
picks one by the network's `protocolVersion`. See the package README.

```sh
pnpm --filter soroban-sign-dialog build
pnpm --filter soroban-sign-dialog demo   # http://127.0.0.1:4178/demo/
```

## Docs

- `docs/architecture.md` — design and state-loading plan.
- `docs/session.md` — network sessions: engine, plan format, CLI, report.
- `docs/rpc-facade.md` — `soroban-sim serve`: the RPC facade, its `fork_*` methods, and rehearsing existing scripts.
- `docs/replay.md` — `soroban-sim replay`: replaying real network transactions to check the session's fidelity.
- `docs/browser-session.md` — network sessions in the browser: the WIT resource, the IndexedDB log, the virtual RPC, the timeline and replay safety.
- `docs/differential-testing.md` — oracles A/B/C and the M3 RPC-equivalence result.
- `docs/upstream.md` — upstream pins and contribution notes.
- `packages/soroban-sign-dialog/README.md` — the sign dialog: embedding, frameworks, theming, the preview engine.

## License

Apache-2.0.
