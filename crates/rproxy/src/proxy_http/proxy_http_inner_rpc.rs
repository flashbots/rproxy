use std::borrow::Cow;

use tracing::warn;

use crate::{
    config::ConfigRpc,
    jrpc::{JrpcRequestMetaMaybeBatch, JrpcResponseMeta},
    proxy::ProxyInner,
    proxy_http::{ProxiedHttpRequest, ProxiedHttpResponse, ProxyHttpInner},
};

const PROXY_HTTP_INNER_RPC_NAME: &str = "rproxy-rpc";

// ProxyHttpInnerRpc ---------------------------------------------------

#[derive(Clone)]
pub(crate) struct ProxyHttpInnerRpc {
    config: ConfigRpc,
}

impl ProxyInner for ProxyHttpInnerRpc {
    #[inline]
    fn name() -> &'static str {
        PROXY_HTTP_INNER_RPC_NAME
    }
}

impl ProxyHttpInner<ConfigRpc> for ProxyHttpInnerRpc {
    fn new(config: ConfigRpc) -> Self {
        Self { config }
    }

    fn config(&self) -> &ConfigRpc {
        &self.config
    }

    fn should_mirror(
        &self,
        jrpc: &JrpcRequestMetaMaybeBatch,
        _: &ProxiedHttpRequest,
        res: &ProxiedHttpResponse,
    ) -> bool {
        fn should_mirror(
            method: Cow<'static, str>,
            res: &JrpcResponseMeta,
            mirror_errored_requests: bool,
        ) -> bool {
            if !matches!(method.as_ref(), "eth_sendRawTransaction" | "eth_sendBundle") {
                return false;
            }

            return mirror_errored_requests || res.error.is_none()
        }

        match jrpc {
            JrpcRequestMetaMaybeBatch::Single(jrpc) => {
                let res = match serde_json::from_slice::<JrpcResponseMeta>(&res.decompressed_body())
                {
                    Ok(jrpc_response) => jrpc_response,
                    Err(err) => {
                        warn!(proxy = Self::name(), error = ?err, "Failed to parse json-rpc response");
                        return false;
                    }
                };

                return should_mirror(
                    jrpc.method.clone(),
                    &res,
                    self.config.mirror_errored_requests,
                );
            }

            JrpcRequestMetaMaybeBatch::Batch(batch) => {
                let res = match serde_json::from_slice::<Vec<JrpcResponseMeta>>(
                    &res.decompressed_body(),
                ) {
                    Ok(jrpc_response) => jrpc_response,
                    Err(err) => {
                        warn!(proxy = Self::name(), error = ?err, "Failed to parse json-rpc response");
                        return false;
                    }
                };

                if res.len() != batch.len() {
                    warn!(
                        proxy = Self::name(),
                        "A response to jrpc-batch has mismatching count of objects (want: {}, got: {})",
                        batch.len(),
                        res.len(),
                    );
                    return false;
                }

                for (idx, jrpc) in batch.iter().enumerate() {
                    if should_mirror(
                        jrpc.method.clone(),
                        &res[idx],
                        self.config.mirror_errored_requests,
                    ) {
                        return true;
                    }
                }
                return false;
            }
        }
    }
}
