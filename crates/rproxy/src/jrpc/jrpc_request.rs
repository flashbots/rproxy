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

#[derive(Debug)]
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
            #[serde(default)]
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

#[derive(Debug, Deserialize)]
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

// tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jrpc_request_meta_maybe_batch_deserialize() {
        let json = r#"[
            {
                "jsonrpc": "2.0",
                "id": 1108954,
                "method": "net_version"
            },
            {
                "jsonrpc": "2.0",
                "id": "1108955",
                "method": "eth_getBlockByNumber",
                "params": [
                    "0x73f151",
                    true
                ]
            }
        ]"#;

        let result: Result<JrpcRequestMetaMaybeBatch, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "{result:?}");

        let batch = result.unwrap();
        match batch {
            JrpcRequestMetaMaybeBatch::Batch(requests) => {
                assert_eq!(requests.len(), 2);

                // First request
                assert_eq!(*requests[0].id(), Id::Number(1108954));
                assert_eq!(requests[0].method(), Cow::Borrowed("net_version"));
                assert!(requests[0].params().is_empty());

                // Second request
                assert_eq!(*requests[1].id(), Id::Number(1108955));
                assert_eq!(requests[1].method(), Cow::Borrowed("eth_getBlockByNumber"));
                assert_eq!(requests[1].params().len(), 2);
            }
            JrpcRequestMetaMaybeBatch::Single(_) => {
                panic!("Expected Batch variant, got Single");
            }
        }
    }
}
