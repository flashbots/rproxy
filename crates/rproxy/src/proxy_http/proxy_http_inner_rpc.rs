use tracing::warn;

use crate::{
    config::ConfigRpc,
    jrpc::JrpcResponseMeta,
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
        jrpc_method: &str,
        _: &ProxiedHttpRequest,
        res: &ProxiedHttpResponse,
    ) -> bool {
        if !matches!(jrpc_method, "eth_sendRawTransaction" | "eth_sendBundle") {
            return false;
        }

        if self.config.mirror_errored_requests {
            return true;
        }

        let jrpc_response =
            match serde_json::from_slice::<JrpcResponseMeta>(&res.decompressed_body()) {
                Ok(jrpc_response) => jrpc_response,
                Err(err) => {
                    warn!(proxy = Self::name(), error = ?err, "Failed to parse json-rpc response");
                    return false;
                }
            };

        return jrpc_response.error.is_none();
    }
}
