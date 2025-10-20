use crate::{
    jrpc::JrpcRequestMetaMaybeBatch,
    server::proxy::{
        ProxyInner,
        http::{ProxiedHttpRequest, ProxiedHttpResponse, config::ConfigProxyHttp},
    },
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
        jrpc_req: &JrpcRequestMetaMaybeBatch,
        http_req: &ProxiedHttpRequest,
        http_res: &ProxiedHttpResponse,
    ) -> bool;
}
