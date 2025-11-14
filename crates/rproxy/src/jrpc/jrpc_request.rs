use std::borrow::Cow;

use alloy_json_rpc::Id;
use serde::Deserialize;

// JrpcRequestMeta -----------------------------------------------------

const JRPC_METHOD_FCUV1_WITH_PAYLOAD: Cow<'static, str> =
    Cow::Borrowed("engine_forkchoiceUpdatedV1_withPayload");
const JRPC_METHOD_FCUV2_WITH_PAYLOAD: Cow<'static, str> =
    Cow::Borrowed("engine_forkchoiceUpdatedV2_withPayload");
const JRPC_METHOD_FCUV3_WITH_PAYLOAD: Cow<'static, str> =
    Cow::Borrowed("engine_forkchoiceUpdatedV3_withPayload");

const EMPTY_PARAMS: &Vec<serde_json::Value> = &Vec::new();

pub(crate) struct JrpcRequestMeta {
    id: Id,

    method: Cow<'static, str>,
    method_enriched: Cow<'static, str>,

    params: serde_json::Value,
}

impl JrpcRequestMeta {
    #[inline]
    pub(crate) fn id(&self) -> &Id {
        &self.id
    }

    #[inline]
    pub(crate) fn method(&self) -> Cow<'static, str> {
        self.method.clone()
    }

    #[inline]
    pub(crate) fn method_enriched(&self) -> Cow<'static, str> {
        self.method_enriched.clone()
    }

    #[inline]
    pub(crate) fn params(&self) -> &Vec<serde_json::Value> {
        self.params.as_array().unwrap_or(EMPTY_PARAMS)
    }
}

impl<'a> Deserialize<'a> for JrpcRequestMeta {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        #[derive(Deserialize)]
        struct JrpcRequestMetaWire {
            id: Id,
            method: Cow<'static, str>,
            params: serde_json::Value,
        }

        let wire = JrpcRequestMetaWire::deserialize(deserializer)?;

        let mut params_count = 0;
        if let Some(params) = wire.params.as_array() {
            for param in params.iter() {
                if !param.is_null() {
                    params_count += 1;
                }
            }
        }

        if params_count < 2 {
            return Ok(Self {
                id: wire.id,
                method: wire.method.clone(),
                method_enriched: wire.method.clone(),
                params: wire.params,
            });
        }

        let method_enriched = match wire.method.as_ref() {
            "engine_forkchoiceUpdatedV1" => JRPC_METHOD_FCUV1_WITH_PAYLOAD.clone(),
            "engine_forkchoiceUpdatedV2" => JRPC_METHOD_FCUV2_WITH_PAYLOAD.clone(),
            "engine_forkchoiceUpdatedV3" => JRPC_METHOD_FCUV3_WITH_PAYLOAD.clone(),

            _ => wire.method.clone(),
        };

        Ok(Self { id: wire.id, method: wire.method, method_enriched, params: wire.params })
    }
}

// JrpcRequestMetaMaybeBatch -------------------------------------------

const JRPC_METHOD_BATCH: Cow<'static, str> = Cow::Borrowed("batch");

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum JrpcRequestMetaMaybeBatch {
    Single(JrpcRequestMeta),
    Batch(Vec<JrpcRequestMeta>),
}

impl JrpcRequestMetaMaybeBatch {
    pub(crate) fn method_enriched(&self) -> Cow<'static, str> {
        match self {
            Self::Single(jrpc) => jrpc.method_enriched.clone(),
            Self::Batch(_) => JRPC_METHOD_BATCH.clone(),
        }
    }
}
