//! Snapshot files: a fork point plus every base entry a session read. A run
//! against RPC can save one (`--save-snapshot`), and a later run can replay
//! the plan from it offline (`--snapshot`), byte for byte the same base state.
//! Keys the snapshot does not hold read as absent.
//!
//! `snapshot export` builds one from a live network (`getLedgerEntries`), and
//! `snapshot import` merges files: both paths reject two different states for
//! the same ledger key, so a snapshot is never ambiguous.

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use soroban_env_host::xdr::{LedgerEntry, LedgerKey, Limits, ReadXdr, WriteXdr};
use soroban_sim_core::session::{ForkPoint, StaticSource, StoredEntry};
use std::collections::BTreeMap;
use std::path::Path;

pub const SNAPSHOT_FORMAT: &str = "soroban-sim-snapshot/1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub format: String,
    pub network_passphrase: String,
    pub fork: SnapshotFork,
    pub entries: Vec<SnapshotEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotFork {
    pub ledger: u32,
    pub close_time: u64,
    pub protocol_version: u32,
    pub base_fee: u32,
    pub base_reserve: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntry {
    /// `LedgerEntry` XDR, base64.
    pub entry: String,
    pub live_until: Option<u32>,
}

impl SnapshotFile {
    pub fn new(passphrase: &str, fork: &ForkPoint, entries: &[StoredEntry]) -> Result<Self> {
        let mut entries: Vec<SnapshotEntry> = entries
            .iter()
            .map(|e| {
                Ok(SnapshotEntry {
                    entry: B64.encode(e.entry.to_xdr(Limits::none())?),
                    live_until: e.live_until,
                })
            })
            .collect::<Result<_>>()?;
        entries.sort_by(|a, b| a.entry.cmp(&b.entry));
        Ok(Self {
            format: SNAPSHOT_FORMAT.into(),
            network_passphrase: passphrase.into(),
            fork: SnapshotFork {
                ledger: fork.ledger_seq,
                close_time: fork.close_time,
                protocol_version: fork.protocol_version,
                base_fee: fork.base_fee,
                base_reserve: fork.base_reserve,
            },
            entries,
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let s: Self =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if s.format != SNAPSHOT_FORMAT {
            bail!(
                "{}: format {:?}, expected {SNAPSHOT_FORMAT:?}",
                path.display(),
                s.format
            );
        }
        Ok(s)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn fork_point(&self) -> ForkPoint {
        ForkPoint {
            ledger_seq: self.fork.ledger,
            close_time: self.fork.close_time,
            protocol_version: self.fork.protocol_version,
            base_reserve: self.fork.base_reserve,
            base_fee: self.fork.base_fee,
        }
    }

    pub fn source(&self) -> Result<StaticSource> {
        let entries = self
            .entries
            .iter()
            .map(|e| {
                Ok(StoredEntry {
                    entry: LedgerEntry::from_xdr(B64.decode(&e.entry)?, Limits::none())?,
                    live_until: e.live_until,
                })
            })
            .collect::<Result<_>>()?;
        StaticSource::new(self.fork.ledger, entries)
    }

    /// Decode every entry and refuse two states for one key. `source` reports
    /// which file or network the snapshot came from in errors.
    pub fn validate(&self, source: &str) -> Result<()> {
        let mut seen: BTreeMap<String, (String, Option<u32>)> = BTreeMap::new();
        for e in &self.entries {
            let entry = LedgerEntry::from_xdr(B64.decode(&e.entry)?, Limits::none())
                .with_context(|| format!("{source}: an entry is not base64 LedgerEntry XDR"))?;
            let key = entry.to_key().to_xdr_base64(Limits::none())?;
            if let Some((existing, live_until)) = seen.get(&key) {
                if *existing != e.entry || *live_until != e.live_until {
                    bail!("{source}: two different states for ledger key {key}");
                }
            } else {
                seen.insert(key, (e.entry.clone(), e.live_until));
            }
        }
        Ok(())
    }
}

/// Merge `snapshots` (name, file) into one canonical file. Every input must
/// describe the same network and fork point; the same key may appear more
/// than once only when it holds exactly the same state.
pub fn merge(snapshots: Vec<(String, SnapshotFile)>) -> Result<SnapshotFile> {
    let Some((first_name, first)) = snapshots.first() else {
        bail!("no snapshots to merge")
    };
    let mut entries: BTreeMap<String, (StoredEntry, String)> = BTreeMap::new();
    for (name, snap) in &snapshots {
        if snap.network_passphrase != first.network_passphrase {
            bail!(
                "{name} is for passphrase {:?}, {first_name} for {:?}: a snapshot holds one network",
                snap.network_passphrase,
                first.network_passphrase
            );
        }
        if snap.fork.ledger != first.fork.ledger
            || snap.fork.close_time != first.fork.close_time
            || snap.fork.protocol_version != first.fork.protocol_version
            || snap.fork.base_fee != first.fork.base_fee
            || snap.fork.base_reserve != first.fork.base_reserve
        {
            bail!("{name} forks at ledger {} but {first_name} at ledger {}: snapshots must describe one ledger", snap.fork.ledger, first.fork.ledger);
        }
        for e in &snap.entries {
            let stored = StoredEntry {
                entry: LedgerEntry::from_xdr(B64.decode(&e.entry)?, Limits::none())
                    .with_context(|| format!("{name}: an entry is not base64 LedgerEntry XDR"))?,
                live_until: e.live_until,
            };
            let key = stored.entry.to_key().to_xdr_base64(Limits::none())?;
            if let Some((existing, from)) = entries.get(&key) {
                if existing.entry.to_xdr(Limits::none())? != stored.entry.to_xdr(Limits::none())?
                    || existing.live_until != stored.live_until
                {
                    bail!("conflicting state for ledger key {key}: {from} and {name}");
                }
            } else {
                entries.insert(key, (stored, name.clone()));
            }
        }
    }
    let stored: Vec<StoredEntry> = entries.into_values().map(|(e, _)| e).collect();
    SnapshotFile::new(&first.network_passphrase, &first.fork_point(), &stored)
}

/// Load and merge snapshot files, with their paths in errors.
pub fn merge_files(paths: &[std::path::PathBuf]) -> Result<SnapshotFile> {
    let mut snapshots = vec![];
    for path in paths {
        let snap = SnapshotFile::load(path)?;
        snap.validate(&path.display().to_string())?;
        snapshots.push((path.display().to_string(), snap));
    }
    merge(snapshots)
}

/// Ledger keys from a file: base64 `LedgerKey` XDR, one per line, or JSON (an
/// array of strings or an object with a `keys` array). `#` starts a comment
/// on a line of its own.
pub fn parse_keys(text: &str, origin: &str) -> Result<Vec<LedgerKey>> {
    let trimmed = text.trim_start();
    let keys: Vec<String> = if trimmed.starts_with('[') || trimmed.starts_with('{') {
        let value: Value =
            serde_json::from_str(text).with_context(|| format!("{origin}: parsing JSON"))?;
        let array = match &value {
            Value::Array(a) => a,
            Value::Object(m) => m
                .get("keys")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("{origin}: JSON object has no keys array"))?,
            _ => bail!("{origin}: expected a JSON array or an object with a keys array"),
        };
        array
            .iter()
            .enumerate()
            .map(|(i, k)| {
                k.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow!("{origin}: keys[{i}] is not a string"))
            })
            .collect::<Result<_>>()?
    } else {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect()
    };
    keys.iter()
        .enumerate()
        .map(|(i, k)| {
            let key = LedgerKey::from_xdr_base64(k.trim(), Limits::none())
                .map_err(|_| anyhow!("{origin}: key {} is not base64 LedgerKey XDR", i + 1))?;
            if matches!(key, LedgerKey::Ttl(_)) {
                bail!(
                    "{origin}: key {} is a LedgerKeyTtl, which RPC cannot query directly",
                    i + 1
                );
            }
            Ok(key)
        })
        .collect()
}
