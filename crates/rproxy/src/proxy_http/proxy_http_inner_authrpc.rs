use std::borrow::Cow;

use crate::{
    config::ConfigAuthrpc,
    jrpc::JrpcRequestMetaMaybeBatch,
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
        jrpc: &JrpcRequestMetaMaybeBatch,
        _: &ProxiedHttpRequest,
        _: &ProxiedHttpResponse,
    ) -> bool {
        fn should_mirror(method: Cow<'static, str>) -> bool {
            if true &&
                !method.starts_with("engine_forkchoiceUpdated") &&
                !method.starts_with("engine_newPayload") &&
                !method.starts_with("miner_setMaxDASize")
            {
                return false;
            }
            return true;
        }

        match jrpc {
            JrpcRequestMetaMaybeBatch::Single(jrpc) => should_mirror(jrpc.method.clone()),

            JrpcRequestMetaMaybeBatch::Batch(batch) => {
                for jrpc in batch.iter() {
                    if should_mirror(jrpc.method.clone()) {
                        return true;
                    }
                }
                return false;
            }
        }
    }
}
