use crate::{
    jrpc::JrpcRequestMetaMaybeBatch,
    server::proxy::http::{ProxiedHttpRequest, ProxiedHttpResponse, config::ConfigProxyHttp},
};

// ProxyHttpInner ------------------------------------------------------

pub(crate) trait ProxyHttpInner<C>: Clone + Send + Sized + Sync + 'static
where
    C: ConfigProxyHttp,
{
    fn name() -> &'static str;
    fn new(config: C) -> Self;
    fn config(&self) -> &C;

    fn should_mirror(
        &self,
        jrpc_req: &JrpcRequestMetaMaybeBatch,
        http_req: &ProxiedHttpRequest,
        http_res: &ProxiedHttpResponse,
    ) -> bool;
}
