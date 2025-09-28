use crate::{
    config::ConfigAuthrpc,
    proxy::ProxyInner,
    proxy_http::{ProxiedHttpRequest, ProxiedHttpResponse, ProxyHttpInner},
};

const PROXY_HTTP_INNER_AUTHRPC_NAME: &str = "rproxy-authrpc";

// ProxyHttpInnerAuthrpc -----------------------------------------------

#[derive(Clone)]
pub(crate) struct ProxyHttpInnerAuthrpc {
    config: ConfigAuthrpc,
}

impl ProxyInner for ProxyHttpInnerAuthrpc {
    #[inline]
    fn name() -> &'static str {
        PROXY_HTTP_INNER_AUTHRPC_NAME
    }
}

impl ProxyHttpInner<ConfigAuthrpc> for ProxyHttpInnerAuthrpc {
    fn new(config: ConfigAuthrpc) -> Self {
        Self { config }
    }

    fn config(&self) -> &ConfigAuthrpc {
        &self.config
    }

    fn should_mirror(
        &self,
        jrpc_method: &str,
        _: &ProxiedHttpRequest,
        _: &ProxiedHttpResponse,
    ) -> bool {
        if true &&
            !jrpc_method.starts_with("engine_forkchoiceUpdated") &&
            !jrpc_method.starts_with("engine_newPayload") &&
            !jrpc_method.starts_with("miner_setMaxDASize")
        {
            return false;
        }

        return true
    }
}
