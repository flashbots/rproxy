use crate::{
    jrpc::{JrpcRequestMeta, JrpcRequestMetaMaybeBatch},
    server::proxy::{
        config::ConfigAuthrpc,
        http::{ProxiedHttpRequest, ProxiedHttpResponse, ProxyHttpInner},
    },
};

const PROXY_HTTP_INNER_AUTHRPC_NAME: &str = "rproxy-authrpc";

// ProxyHttpInnerAuthrpc -----------------------------------------------

#[derive(Clone)]
pub(crate) struct ProxyHttpInnerAuthrpc {
    config: ConfigAuthrpc,
}

impl ProxyHttpInner<ConfigAuthrpc> for ProxyHttpInnerAuthrpc {
    #[inline]
    fn name() -> &'static str {
        PROXY_HTTP_INNER_AUTHRPC_NAME
    }

    fn new(config: ConfigAuthrpc) -> Self {
        Self { config }
    }

    fn config(&self) -> &ConfigAuthrpc {
        &self.config
    }

    fn should_mirror(
        &self,
        jrpc_req: &JrpcRequestMetaMaybeBatch,
        _: &ProxiedHttpRequest,
        _: &ProxiedHttpResponse,
    ) -> bool {
        fn should_mirror(jrpc_req: &JrpcRequestMeta) -> bool {
            let method = jrpc_req.method();

            if !method.starts_with("engine_forkchoiceUpdated") &&
                !method.starts_with("engine_newPayload") &&
                !method.starts_with("miner_setMaxDASize")
            {
                return false;
            }
            true
        }

        match jrpc_req {
            JrpcRequestMetaMaybeBatch::Single(jrpc_req_single) => should_mirror(jrpc_req_single),

            JrpcRequestMetaMaybeBatch::Batch(jrpc_req_batch) => {
                for jrpc_req_single in jrpc_req_batch.iter() {
                    if should_mirror(jrpc_req_single) {
                        return true;
                    }
                }
                false
            }
        }
    }
}
