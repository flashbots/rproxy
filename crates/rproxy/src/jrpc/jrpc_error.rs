use serde::Deserialize;

// JrpcError -----------------------------------------------------------

#[derive(Clone, Deserialize)]
pub(crate) struct JrpcError {
    // pub(crate) code: i64,
    // pub(crate) message: String,
}
