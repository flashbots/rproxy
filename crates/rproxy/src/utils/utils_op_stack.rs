use alloy_rlp::Decodable;
use hex::FromHex;

// raw_transaction_to_hash ---------------------------------------------

pub(crate) fn raw_transaction_to_hash(transaction: &mut serde_json::Value) {
    let Some(hex) = transaction.as_str() else {
        return;
    };
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    let Ok(bytes) = Vec::from_hex(hex) else {
        return;
    };
    let mut buf = bytes.as_slice();
    let Ok(envelope) = op_alloy_consensus::OpTxEnvelope::decode(&mut buf) else {
        return;
    };
    let hash = envelope.hash().to_string();
    *transaction = serde_json::Value::String(hash);
}
