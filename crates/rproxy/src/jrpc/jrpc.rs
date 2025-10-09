use std::borrow::Cow;

use serde::Deserialize;

// JrpcError -----------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct JrpcError {
    // pub(crate) code: i64,
    // pub(crate) message: String,
}

// JrpcRequestMeta -----------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct JrpcRequestMeta {
    pub(crate) method: Cow<'static, str>,
}

// JrpcRequestMetaMaybeBatch -------------------------------------------

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum JrpcRequestMetaMaybeBatch {
    Single(JrpcRequestMeta),
    Batch(Vec<JrpcRequestMeta>),
}

impl JrpcRequestMetaMaybeBatch {
    pub(crate) fn method(&self) -> Cow<'static, str> {
        match self {
            Self::Single(jrpc) => jrpc.method.clone(),
            Self::Batch(_) => Cow::Borrowed("batch"),
        }
    }
}

// JrpcResponseMeta ----------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct JrpcResponseMeta {
    pub(crate) error: Option<JrpcError>,
}
