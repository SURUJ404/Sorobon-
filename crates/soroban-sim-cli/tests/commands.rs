//! The one-shot commands, offline: `simulate`, `fork`, and
//! `snapshot export`/`import` over the committed parity fixture.
//!
//! The fixture records what stellar-rpc's `simulateTransaction` answered for
//! one local envelope, together with the exact ledger entries it ran against.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::Value;
use soroban_env_host::xdr::{
    LedgerEntry, Limits, ReadXdr, ScVal, SorobanTransactionData, WriteXdr,
};
use soroban_sim_cli::fork;
use soroban_sim_cli::run::{Base, ForkMode};
use soroban_sim_cli::simulate;
use soroban_sim_cli::snapshot::{self, SnapshotFile};
use soroban_sim_cli::snapshot_export::{self, ExportOptions};
use soroban_sim_core::session::{host_protocol, ForkPoint, StoredEntry};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "soroban-sim-commands-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixture() -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(repo().join("fixtures/parity/local-add-i32.json")).unwrap(),
    )
    .unwrap()
}

fn fork_point(fixture: &Value) -> ForkPoint {
    let c = &fixture["ledgerContext"];
    ForkPoint {
        ledger_seq: c["sequenceNumber"].as_u64().unwrap() as u32,
        close_time: c["timestamp"].as_u64().unwrap(),
        protocol_version: c["protocolVersion"].as_u64().unwrap() as u32,
        base_reserve: c["baseReserve"].as_u64().unwrap() as u32,
        base_fee: 100,
    }
}

fn entries(fixture: &Value) -> Vec<StoredEntry> {
    fixture["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| StoredEntry {
            entry: LedgerEntry::from_xdr(
                B64.decode(e["entryXdr"].as_str().unwrap()).unwrap(),
                Limits::none(),
            )
            .unwrap(),
            live_until: e["liveUntilLedger"].as_u64().map(|v| v as u32),
        })
        .collect()
}

fn snapshot_at(
    dir: &Path,
    name: &str,
    fixture: &Value,
    entries: &[StoredEntry],
    fork: &ForkPoint,
) -> PathBuf {
    let path = dir.join(name);
    SnapshotFile::new(
        fixture["networkPassphrase"].as_str().unwrap(),
        fork,
        entries,
    )
    .unwrap()
    .save(&path)
    .unwrap();
    path
}

#[test]
fn simulate_matches_the_recorded_rpc_result() {
    let dir = scratch("simulate");
    let fixture = fixture();
    let fork = fork_point(&fixture);
    let path = snapshot_at(&dir, "base.json", &fixture, &entries(&fixture), &fork);

    let result = simulate::run(Base::Snapshot(path), fixture["txXdr"].as_str().unwrap()).unwrap();
    assert!(
        result.get("error").is_none(),
        "simulation failed: {}",
        result["error"]
    );
    let expected = &fixture["rpc"];
    assert_eq!(
        result["results"][0]["xdr"], expected["retvalXdr"],
        "return value"
    );
    assert_eq!(
        result["minResourceFee"], expected["minResourceFee"],
        "resource fee"
    );
    // `cost` is the raw metered count; the transaction data carries the
    // stellar-rpc default padding (raw + 50_000).
    let recorded = expected["instructions"].as_u64().unwrap();
    assert_eq!(
        result["cost"]["cpuInsns"].as_str().unwrap(),
        (recorded - 50_000).to_string(),
        "metered instructions"
    );
    let data = SorobanTransactionData::from_xdr_base64(
        result["transactionData"].as_str().unwrap(),
        Limits::none(),
    )
    .unwrap();
    assert_eq!(
        data.resources.instructions as u64, recorded,
        "padded instructions"
    );
    let mut read_only: Vec<String> = data
        .resources
        .footprint
        .read_only
        .iter()
        .map(|k| k.to_xdr_base64(Limits::none()).unwrap())
        .collect();
    read_only.sort();
    let mut expected_read_only: Vec<String> = expected["footprintReadOnly"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap().to_string())
        .collect();
    expected_read_only.sort();
    assert_eq!(read_only, expected_read_only, "read-only footprint");
    assert_eq!(
        data.resources.footprint.read_write.len(),
        expected["footprintReadWrite"].as_array().unwrap().len(),
        "read-write footprint"
    );
    assert_eq!(
        result["events"].as_array().unwrap().len(),
        expected["events"].as_array().unwrap().len(),
        "events"
    );
    let value = ScVal::from_xdr_base64(
        result["results"][0]["xdr"].as_str().unwrap(),
        Limits::none(),
    )
    .unwrap();
    assert_eq!(value, ScVal::I32(42));
    assert!(simulate::summary(&result).unwrap().contains("result: 42"));
}

#[test]
fn simulate_reports_a_bad_envelope_as_an_error_not_a_panic() {
    assert!(simulate::run(
        Base::Snapshot(PathBuf::from("does-not-matter.json")),
        "not base64"
    )
    .is_err());
}

#[test]
fn fork_reports_the_snapshot_ledger_and_protocol() {
    let dir = scratch("fork");
    let fixture = fixture();
    let fork = fork_point(&fixture);
    let path = snapshot_at(&dir, "base.json", &fixture, &entries(&fixture), &fork);

    let info = fork::inspect(Base::Snapshot(path), ForkMode::Head).unwrap();
    assert_eq!(
        info["network"].as_str().unwrap(),
        format!("snapshot {}", dir.join("base.json").display())
    );
    assert_eq!(info["forkLedger"], fork.ledger_seq);
    assert_eq!(info["protocol"], host_protocol());
    assert_eq!(
        info["passphrase"],
        fixture["networkPassphrase"].as_str().unwrap()
    );
    assert_eq!(info["baseFee"], 100);
    assert!(fork::summary(&info).contains(&format!("at ledger {}", fork.ledger_seq)));
}

#[test]
fn snapshot_export_round_trips_a_transaction_footprint() {
    let dir = scratch("export");
    let fixture = fixture();
    let fork = fork_point(&fixture);
    let source = snapshot_at(&dir, "source.json", &fixture, &entries(&fixture), &fork);

    let report = snapshot_export::export(ExportOptions {
        base: Base::Snapshot(source.clone()),
        fork_mode: ForkMode::Head,
        envelope: Some(fixture["txXdr"].as_str().unwrap().to_string()),
        keys_file: None,
        keys: vec![],
    })
    .unwrap();
    assert_eq!(report.ledger, fork.ledger_seq);
    assert_eq!(report.protocol, host_protocol());
    assert!(
        report.requested >= 2,
        "the source account and the footprint"
    );
    assert!(report.snapshot.entries.len() >= report.requested);

    let exported = dir.join("exported.json");
    report.snapshot.save(&exported).unwrap();
    let result =
        simulate::run(Base::Snapshot(exported), fixture["txXdr"].as_str().unwrap()).unwrap();
    assert!(
        result.get("error").is_none(),
        "exported snapshot does not replay: {}",
        result["error"]
    );
    assert_eq!(result["results"][0]["xdr"], fixture["rpc"]["retvalXdr"]);
}

#[test]
fn snapshot_export_reads_explicit_keys() {
    let dir = scratch("export-keys");
    let fixture = fixture();
    let fork = fork_point(&fixture);
    let source = snapshot_at(&dir, "source.json", &fixture, &entries(&fixture), &fork);
    let key = fixture["snapshot"][0]["keyXdr"]
        .as_str()
        .unwrap()
        .to_string();

    let report = snapshot_export::export(ExportOptions {
        base: Base::Snapshot(source),
        fork_mode: ForkMode::Head,
        envelope: None,
        keys_file: None,
        keys: vec![key],
    })
    .unwrap();
    assert_eq!(report.requested, 1);
    // Session::new also read the network settings, so a bare snapshot is still
    // enough to open offline.
    assert!(report.snapshot.entries.len() > 1);
}

#[test]
fn snapshot_import_merges_and_canonicalizes() {
    let dir = scratch("import");
    let fixture = fixture();
    let fork = fork_point(&fixture);
    let all = entries(&fixture);
    assert!(all.len() >= 4);
    let (a, b) = all.split_at(all.len() / 2);
    // Split the same entries across two files, in opposite orders.
    let first = snapshot_at(&dir, "a.json", &fixture, a, &fork);
    let second = snapshot_at(&dir, "b.json", &fixture, b, &fork);

    let merged = snapshot::merge_files(&[first.clone(), second]).unwrap();
    assert_eq!(merged.entries.len(), all.len());
    let reread = SnapshotFile::load(&first).unwrap();
    assert_eq!(merged.fork.ledger, reread.fork.ledger);
    assert_eq!(merged.network_passphrase, reread.network_passphrase);
    // Canonical: entries sorted by XDR, and identical duplicates are deduped.
    let mut sorted = merged.entries.clone();
    sorted.sort_by(|a, b| a.entry.cmp(&b.entry));
    assert_eq!(
        merged.entries.iter().map(|e| &e.entry).collect::<Vec<_>>(),
        sorted.iter().map(|e| &e.entry).collect::<Vec<_>>()
    );
    let again = snapshot::merge_files(&[first.clone(), first]).unwrap();
    assert_eq!(
        again.entries.len(),
        all.len() / 2,
        "a file merged with itself holds its own half"
    );
}

#[test]
fn snapshot_import_refuses_conflicting_state() {
    let dir = scratch("conflict");
    let fixture = fixture();
    let fork = fork_point(&fixture);
    let all = entries(&fixture);
    let first = snapshot_at(&dir, "a.json", &fixture, &all[..1], &fork);

    let mut changed = all[0].clone();
    changed.entry.last_modified_ledger_seq += 1;
    let second = snapshot_at(&dir, "b.json", &fixture, &[changed], &fork);
    let error = snapshot::merge_files(&[first, second])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("conflicting state for ledger key"),
        "{error}"
    );

    let other_fork = ForkPoint {
        ledger_seq: fork.ledger_seq + 1,
        ..fork.clone()
    };
    let third = snapshot_at(&dir, "c.json", &fixture, &all[..1], &other_fork);
    let error = snapshot::merge_files(&[
        snapshot_at(&dir, "a.json", &fixture, &all[..1], &fork),
        third,
    ])
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("snapshots must describe one ledger"),
        "{error}"
    );
}

#[test]
fn snapshot_import_refuses_another_network() {
    let dir = scratch("network");
    let fixture = fixture();
    let fork = fork_point(&fixture);
    let all = entries(&fixture);
    let a = snapshot_at(&dir, "a.json", &fixture, &all[..1], &fork);
    let other = dir.join("b.json");
    SnapshotFile::new("another network", &fork, &all[1..2])
        .unwrap()
        .save(&other)
        .unwrap();
    let error = snapshot::merge_files(&[a, other]).unwrap_err().to_string();
    assert!(error.contains("a snapshot holds one network"), "{error}");
}

#[test]
fn snapshot_keys_parse_text_json_and_comments() {
    let fixture = fixture();
    let key = fixture["snapshot"][0]["keyXdr"].as_str().unwrap();
    let text = format!("# exported keys\n\n{key}\n");
    assert_eq!(snapshot::parse_keys(&text, "test").unwrap().len(), 1);
    assert_eq!(
        snapshot::parse_keys(&format!("[{key:?}]"), "test")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        snapshot::parse_keys(&format!("{{\"keys\":[{key:?}]}}"), "test")
            .unwrap()
            .len(),
        1
    );
    assert!(snapshot::parse_keys("not-base64", "test")
        .unwrap_err()
        .to_string()
        .contains("not base64 LedgerKey"));
}

#[test]
fn snapshot_validation_rejects_garbled_entries() {
    let fixture = fixture();
    let fork = fork_point(&fixture);
    let mut snap = SnapshotFile::new(
        fixture["networkPassphrase"].as_str().unwrap(),
        &fork,
        &entries(&fixture),
    )
    .unwrap();
    snap.entries[0].entry = "AAAA".into();
    assert!(snap
        .validate("test")
        .unwrap_err()
        .to_string()
        .contains("not base64 LedgerEntry XDR"));
    // The fixture's entries themselves are valid and distinct.
    let good = SnapshotFile::new(
        fixture["networkPassphrase"].as_str().unwrap(),
        &fork,
        &entries(&fixture),
    )
    .unwrap();
    good.validate("fixture").unwrap();
}

/// The binary itself: argument parsing, host selection, dispatch and exit
/// status, over the same offline fixture.
#[test]
fn the_cli_runs_simulate_fork_and_snapshot_import() {
    let dir = scratch("cli");
    let fixture = fixture();
    let fork = fork_point(&fixture);
    let all = entries(&fixture);
    let base = snapshot_at(&dir, "base.json", &fixture, &all, &fork);
    let binary = env!("CARGO_BIN_EXE_soroban-sim");

    let output = std::process::Command::new(binary)
        .args([
            "simulate",
            "--snapshot",
            base.to_str().unwrap(),
            "--json",
            fixture["txXdr"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let simulated: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        simulated["results"][0]["xdr"],
        fixture["rpc"]["retvalXdr"],
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = std::process::Command::new(binary)
        .args(["fork", "--snapshot", base.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info["forkLedger"], fork.ledger_seq);
    assert_eq!(info["protocol"], host_protocol());

    let half = all.len() / 2;
    let a = snapshot_at(&dir, "a.json", &fixture, &all[..half], &fork);
    let b = snapshot_at(&dir, "b.json", &fixture, &all[half..], &fork);
    let merged = dir.join("merged.json");
    let output = std::process::Command::new(binary)
        .args([
            "snapshot",
            "import",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--out",
            merged.to_str().unwrap(),
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        SnapshotFile::load(&merged).unwrap().entries.len(),
        all.len()
    );
}

#[test]
fn the_cli_exit_codes_distinguish_a_failed_simulation_from_bad_input() {
    let dir = scratch("exit-codes");
    let fixture = fixture();
    let fork = fork_point(&fixture);
    // Only the network settings: the contract the envelope calls is absent.
    let config: Vec<StoredEntry> = entries(&fixture)
        .into_iter()
        .filter(|e| {
            matches!(
                e.entry.data,
                soroban_env_host::xdr::LedgerEntryData::ConfigSetting(_)
            )
        })
        .collect();
    assert!(!config.is_empty());
    let base = snapshot_at(&dir, "config-only.json", &fixture, &config, &fork);
    let binary = env!("CARGO_BIN_EXE_soroban-sim");

    let output = std::process::Command::new(binary)
        .args([
            "simulate",
            "--snapshot",
            base.to_str().unwrap(),
            "--json",
            fixture["txXdr"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "a simulation error is exit 1: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let simulated: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(simulated["error"].as_str().is_some(), "{simulated}");

    let output = std::process::Command::new(binary)
        .args([
            "simulate",
            "--snapshot",
            base.to_str().unwrap(),
            "not-base64",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "bad input is exit 2: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a base64 TransactionEnvelope"));
}
