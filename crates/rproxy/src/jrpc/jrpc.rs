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

// JrpcResponseMeta ----------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct JrpcResponseMeta {
    pub(crate) error: Option<JrpcError>,
}
