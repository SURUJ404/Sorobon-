//! Reading a transaction envelope from the places a command line user keeps
//! one: a file, stdin (`-`), a literal base64 string, or JSON with a
//! `transaction`, `envelope`, `xdr` or `tx` field.

use anyhow::{anyhow, bail, Context, Result};
use soroban_env_host::xdr::{Limits, ReadXdr, TransactionEnvelope, WriteXdr};
use std::io::Read;
use std::path::Path;

/// The envelope's canonical base64 XDR.
pub fn envelope_xdr(input: &str) -> Result<String> {
    let text = if input == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading the envelope from stdin")?;
        text
    } else if Path::new(input).is_file() {
        std::fs::read_to_string(input)
            .with_context(|| format!("reading the envelope from {input}"))?
    } else {
        input.to_string()
    };
    let text = text.trim();
    let text = if text.starts_with('{') {
        let value: serde_json::Value =
            serde_json::from_str(text).context("parsing envelope JSON")?;
        ["transaction", "envelope", "xdr", "tx"]
            .iter()
            .find_map(|k| value.get(*k).and_then(serde_json::Value::as_str))
            .map(str::trim)
            .map(str::to_string)
            .ok_or_else(|| {
                anyhow!("JSON envelope has no transaction, envelope, xdr or tx string")
            })?
    } else {
        text.to_string()
    };
    let text = text.trim();
    if text.is_empty() {
        bail!("the envelope is empty");
    }
    let envelope = TransactionEnvelope::from_xdr_base64(text, Limits::none())
        .map_err(|e| anyhow!("not a base64 TransactionEnvelope: {e}"))?;
    envelope
        .to_xdr_base64(Limits::none())
        .context("re-encoding the envelope")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_literal_and_json_base64() {
        let fixture = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/fixtures/parity/local-add-i32.json"
            ))
            .unwrap(),
        )
        .unwrap();
        let xdr = fixture["txXdr"].as_str().unwrap();
        assert_eq!(envelope_xdr(xdr).unwrap(), xdr);
        assert_eq!(
            envelope_xdr(&format!("{{\"transaction\":{xdr:?}}}")).unwrap(),
            xdr
        );
        assert_eq!(envelope_xdr(&format!("{{\"xdr\":{xdr:?}}}")).unwrap(), xdr);
    }

    #[test]
    fn refuses_garbage_and_empty_json() {
        assert!(envelope_xdr("not-an-envelope")
            .unwrap_err()
            .to_string()
            .contains("not a base64"));
        assert_eq!(
            envelope_xdr("{}").unwrap_err().to_string(),
            "JSON envelope has no transaction, envelope, xdr or tx string"
        );
    }
}
