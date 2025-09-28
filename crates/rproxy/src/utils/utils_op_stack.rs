use alloy_rlp::Decodable;
use hex::FromHex;

// raw_transaction_to_hash ---------------------------------------------

pub fn raw_transaction_to_hash(transaction: &mut serde_json::Value) {
    let hex = match transaction.as_str() {
        Some(hex) => hex,
        None => return,
    };
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    let bytes = match Vec::from_hex(hex) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    let mut buf = bytes.as_slice();
    let envelope = match op_alloy_consensus::OpTxEnvelope::decode(&mut buf) {
        Ok(envelope) => envelope,
        Err(_) => return,
    };
    let hash = envelope.hash().to_string();
    *transaction = serde_json::Value::String(hash);
}
