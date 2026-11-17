//! `soroban-sim snapshot export`: build a snapshot file from a live fork.
//!
//! With `--transaction`, the envelope's own keys plus the footprint the
//! simulation discovers are read at the fork point. With `--keys`/`--key`,
//! exactly those ledger keys are read. The session also records the network
//! settings every offline run needs, so the result replays without RPC.

use crate::input::envelope_xdr;
use crate::run::{fork_base, Base, ForkMode};
use crate::snapshot::{parse_keys, SnapshotFile};
use anyhow::{anyhow, bail, Context, Result};
use soroban_env_host::e2e_invoke::RecordingInvocationAuthMode;
use soroban_env_host::xdr::{
    LedgerKey, Limits, OperationBody, ReadXdr, TransactionEnvelope, WriteXdr,
};
use soroban_sim_core::session::{muxed_account_id, SessionOptions};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub struct ExportOptions {
    pub base: Base,
    pub fork_mode: ForkMode,
    /// An envelope whose footprint to export.
    pub envelope: Option<String>,
    /// A file of base64 `LedgerKey` XDR.
    pub keys_file: Option<PathBuf>,
    /// Base64 `LedgerKey` XDR values.
    pub keys: Vec<String>,
}

pub struct ExportReport {
    pub snapshot: SnapshotFile,
    pub network: String,
    pub base: String,
    pub ledger: u32,
    pub protocol: u32,
    /// Distinct ledger keys asked for (envelope keys included).
    pub requested: usize,
    /// Entries present at the fork point (absent keys are not stored).
    pub found: usize,
}

pub fn export(options: ExportOptions) -> Result<ExportReport> {
    let envelope = options.envelope.as_deref().map(envelope_xdr).transpose()?;
    let mut keys: BTreeMap<String, LedgerKey> = BTreeMap::new();
    let mut add = |key: LedgerKey, source: &str| -> Result<()> {
        if matches!(key, LedgerKey::Ttl(_)) {
            bail!("{source}: LedgerKeyTtl entries cannot be queried directly");
        }
        keys.insert(key.to_xdr_base64(Limits::none())?, key);
        Ok(())
    };

    for (i, text) in options.keys.iter().enumerate() {
        let key = LedgerKey::from_xdr_base64(text.trim(), Limits::none())
            .map_err(|_| anyhow!("--key {}: not base64 LedgerKey XDR", i + 1))?;
        add(key, &format!("--key {}", i + 1))?;
    }
    if let Some(path) = &options.keys_file {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        for key in parse_keys(&text, &path.display().to_string())? {
            add(key, &path.display().to_string())?;
        }
    }

    let base = options.base.with_mode(options.fork_mode)?;
    let forked = fork_base(base, &SessionOptions::new(""))?;
    let session = (forked.fresh)().context("opening the fork")?;

    if let Some(xdr) = &envelope {
        let envelope = TransactionEnvelope::from_xdr_base64(xdr, Limits::none())?;
        let inner = soroban_sim_facade::tx::inner_transaction(&envelope);
        for key in soroban_sim_facade::tx::envelope_keys(&inner) {
            add(key, "the envelope")?;
        }
        if let TransactionEnvelope::TxFeeBump(fee_bump) = &envelope {
            add(
                soroban_sim_core::session::account_key(&muxed_account_id(&fee_bump.tx.fee_source)),
                "the fee source",
            )?;
        }
        let [op] = inner.operations.as_slice() else {
            bail!("the envelope must carry exactly one operation")
        };
        let OperationBody::InvokeHostFunction(ihf) = &op.body else {
            bail!("snapshot export simulates InvokeHostFunction operations only (this envelope has {})", op.body.name())
        };
        let source = op
            .source_account
            .as_ref()
            .map(muxed_account_id)
            .unwrap_or_else(|| muxed_account_id(&inner.source_account));
        let auth = if ihf.auth.is_empty() {
            RecordingInvocationAuthMode::recording(true, false)
        } else {
            // The envelope carries its own signed entries: check them, as RPC
            // does by default, so the exported footprint is the one it will use.
            RecordingInvocationAuthMode::Enforcing(ihf.auth.to_vec())
        };
        let simulated = session
            .simulate(&source, &ihf.host_function, auth)
            .with_context(|| "simulating the envelope to discover its footprint")?;
        let data = simulated.transaction_data.as_ref().ok_or_else(|| {
            anyhow!("simulation returned no transaction data, so there is no footprint to export")
        })?;
        for key in data
            .resources
            .footprint
            .read_only
            .iter()
            .chain(data.resources.footprint.read_write.iter())
        {
            add(key.clone(), "the simulated footprint")?;
        }
        if let Err(e) = &simulated.invoke_result {
            eprintln!(
                "warning: the simulated call fails ({:?}); exporting the footprint it still read",
                e.error
            );
        }
    }

    if keys.is_empty() {
        bail!("nothing to export: pass --transaction ENVELOPE, --keys FILE, or --key BASE64");
    }

    let requested = keys.len();
    session
        .ledger()
        .prefetch(&keys.into_values().collect::<Vec<_>>())?;
    let entries = session.ledger().base_entries();
    let found = entries.len();
    let snapshot = SnapshotFile::new(&forked.passphrase, session.fork_point(), &entries)?;
    Ok(ExportReport {
        snapshot,
        network: forked.network_name,
        base: forked.description,
        ledger: session.fork_point().ledger_seq,
        protocol: session.fork_point().protocol_version,
        requested,
        found,
    })
}
