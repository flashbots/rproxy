use serde::Deserialize;

use crate::jrpc::JrpcError;

// JrpcResponseMeta ----------------------------------------------------

#[derive(Clone, Deserialize)]
pub(crate) struct JrpcResponseMeta {
    pub(crate) error: Option<JrpcError>,
}
