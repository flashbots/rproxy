use crate::{
    config::ConfigProxyHttp,
    jrpc::JrpcRequestMetaMaybeBatch,
    proxy::ProxyInner,
    proxy_http::{ProxiedHttpRequest, ProxiedHttpResponse},
};

// ProxyHttpInner ------------------------------------------------------

pub(crate) trait ProxyHttpInner<C>:
    ProxyInner + Clone + Send + Sized + Sync + 'static
where
    C: ConfigProxyHttp,
{
    fn new(config: C) -> Self;
    fn config(&self) -> &C;

    fn should_mirror(
        &self,
        jrpc: &JrpcRequestMetaMaybeBatch,
        req: &ProxiedHttpRequest,
        res: &ProxiedHttpResponse,
    ) -> bool;
}
