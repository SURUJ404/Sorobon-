//! `soroban-sim simulate`: one envelope through the same
//! `simulateTransaction` path the RPC facade serves, over a forked network or
//! an offline snapshot. The response is stellar-rpc's shape, so callers can
//! use it in place of a network simulation without a running facade.

use crate::input::envelope_xdr;
use crate::run::Base;
use crate::serve::{Facade, ServeConfig};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use soroban_env_host::xdr::{Limits, ReadXdr, ScVal, SorobanTransactionData};
use soroban_sim_core::session::show_scval;

/// Simulate `input` (see [`envelope_xdr`]) and return the `simulateTransaction`
/// result: `error` plus `latestLedger` on failure, or the resource data,
/// result value, auth, events and state changes on success.
pub fn run(base: Base, input: &str) -> Result<Value> {
    let xdr = envelope_xdr(input)?;
    let mut facade = Facade::open(ServeConfig {
        base,
        seconds_per_ledger: None,
        // Simulation never applies or signs anything: the flag only gates
        // sendTransaction on mainnet.
        allow_real_signers: true,
        report: None,
        progress: false,
        verbose: false,
    })?;
    facade
        .call("simulateTransaction", json!({ "transaction": xdr }))
        .map_err(|e| anyhow!("simulateTransaction: {}", e.message))
}

/// A few lines for a terminal. The raw JSON stays available with `--json`.
pub fn summary(result: &Value) -> Result<String> {
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return Ok(format!("simulation failed: {error}"));
    }
    let latest = result["latestLedger"].as_u64().unwrap_or(0);
    let value = result["results"]
        .as_array()
        .and_then(|r| r.first())
        .and_then(|r| r["xdr"].as_str())
        .ok_or_else(|| anyhow!("simulateTransaction returned no results[0].xdr"))?;
    let value = ScVal::from_xdr_base64(value, Limits::none())?;
    let fee = result["minResourceFee"].as_str().unwrap_or("0");
    let cpu = result["cost"]["cpuInsns"].as_str().unwrap_or("0");
    let mem = result["cost"]["memBytes"].as_str().unwrap_or("0");
    let auth = result["results"]
        .as_array()
        .and_then(|r| r.first())
        .and_then(|r| r["auth"].as_array())
        .map_or(0, Vec::len);
    let events = result["events"].as_array().map_or(0, Vec::len);
    let changes = result["stateChanges"].as_array().map_or(0, Vec::len);
    let resources = result["transactionData"]
        .as_str()
        .and_then(|x| SorobanTransactionData::from_xdr_base64(x, Limits::none()).ok())
        .map(|d| {
            format!(
                "  resources: {} instructions (metered {cpu}); memory {mem} bytes; read {} / write {} bytes",
                d.resources.instructions, d.resources.disk_read_bytes, d.resources.write_bytes
            )
        })
        .unwrap_or_else(|| format!("  instructions: {cpu}; memory: {mem} bytes"));
    Ok(format!(
        "simulated in ledger {latest}: ok\n  result: {}\n  min resource fee: {fee} stroops\n{resources}\n  auth: {auth}; events: {events}; state changes: {changes}",
        show_scval(&value)
    ))
}
