//! `soroban-sim fork`: open a fork and describe exactly what it is, without
//! running a plan or serving RPC. Useful for checking which ledger a
//! checkpoint pins, whether a snapshot matches the network, and which host
//! protocol a build will use.

use crate::run::{fork_base, Base, ForkMode};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use soroban_sim_core::session::{host_protocol, network_id, SessionOptions};
use soroban_sim_facade::report::hex;

/// Open `base` in `mode` and return its identity as JSON.
pub fn inspect(base: Base, mode: ForkMode) -> Result<Value> {
    let base = base.with_mode(mode)?;
    let forked = fork_base(base, &SessionOptions::new(""))?;
    let session = (forked.fresh)().context("opening the fork")?;
    let fork = session.fork_point();
    let info = session.ledger_info();
    Ok(json!({
        "network": forked.network_name,
        "passphrase": forked.passphrase,
        "base": forked.description,
        "forkLedger": fork.ledger_seq,
        "closeTime": fork.close_time,
        "protocol": fork.protocol_version,
        "baseFee": fork.base_fee,
        "baseReserve": fork.base_reserve,
        "networkId": hex(&network_id(&forked.passphrase)),
        "hostProtocol": host_protocol(),
        "sessionLedger": info.sequence_number,
        "sessionCloseTime": info.timestamp,
    }))
}

/// A few lines for a terminal.
pub fn summary(info: &Value) -> String {
    let text = |key: &str| match info.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => "-".into(),
    };
    format!(
        "{} at ledger {} (protocol {}, base fee {} stroops, base reserve {} stroops)\n  passphrase: {:?}\n  network id: {}\n  base: {}\n  session starts at ledger {}, close time {}",
        text("network"),
        text("forkLedger"),
        text("protocol"),
        text("baseFee"),
        text("baseReserve"),
        text("passphrase"),
        text("networkId"),
        text("base"),
        text("sessionLedger"),
        text("sessionCloseTime"),
    )
}
