use tracing::warn;

use crate::{
    jrpc::{JrpcError, JrpcRequestMeta, JrpcRequestMetaMaybeBatch, JrpcResponseMeta},
    server::proxy::{
        ProxyInner,
        config::ConfigRpc,
        http::{ProxiedHttpRequest, ProxiedHttpResponse, ProxyHttpInner},
    },
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
        jrpc_req: &JrpcRequestMetaMaybeBatch,
        _: &ProxiedHttpRequest,
        http_res: &ProxiedHttpResponse,
    ) -> bool {
        fn should_mirror(
            jrpc_req: &JrpcRequestMeta,
            jrpc_res: &JrpcResponseMeta,
            mirror_errored_requests: bool,
        ) -> bool {
            if !matches!(jrpc_req.method().as_ref(), "eth_sendRawTransaction" | "eth_sendBundle") {
                return false;
            }

            mirror_errored_requests || jrpc_res.error.is_none()
        }

        match jrpc_req {
            JrpcRequestMetaMaybeBatch::Single(jrpc_req_single) => {
                let jrpc_res_single = match serde_json::from_slice::<JrpcResponseMeta>(
                    &http_res.decompressed_body(),
                ) {
                    Ok(jrpc_response) => jrpc_response,
                    Err(err) => {
                        warn!(proxy = Self::name(), error = ?err, "Failed to parse json-rpc response");

                        return should_mirror(
                            jrpc_req_single,
                            &JrpcResponseMeta { error: Some(JrpcError {}) },
                            self.config.mirror_errored_requests,
                        );
                    }
                };

                should_mirror(
                    jrpc_req_single,
                    &jrpc_res_single,
                    self.config.mirror_errored_requests,
                )
            }

            JrpcRequestMetaMaybeBatch::Batch(jrpc_req_batch) => {
                let mut jrpc_res_batch = match serde_json::from_slice::<Vec<JrpcResponseMeta>>(
                    &http_res.decompressed_body(),
                ) {
                    Ok(jrpc_response) => jrpc_response,
                    Err(err) => {
                        warn!(proxy = Self::name(), error = ?err, "Failed to parse json-rpc response");
                        vec![JrpcResponseMeta { error: Some(JrpcError {}) }; jrpc_req_batch.len()]
                    }
                };

                if jrpc_res_batch.len() != jrpc_req_batch.len() {
                    warn!(
                        proxy = Self::name(),
                        "A response to jrpc-batch has mismatching count of objects (want: {}, got: {})",
                        jrpc_req_batch.len(),
                        jrpc_res_batch.len(),
                    );
                    jrpc_res_batch =
                        vec![JrpcResponseMeta { error: Some(JrpcError {}) }; jrpc_req_batch.len()];
                }

                for (idx, jrpc_req_single) in jrpc_req_batch.iter().enumerate() {
                    let jrpc_res_single = &jrpc_res_batch[idx];
                    if should_mirror(
                        jrpc_req_single,
                        jrpc_res_single,
                        self.config.mirror_errored_requests,
                    ) {
                        return true;
                    }
                }
                false
            }
        }
    }
}
