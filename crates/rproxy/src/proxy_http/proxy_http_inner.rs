use std::borrow::Cow;

use crate::{
    config::ConfigProxyHttp,
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
        jrpc_method: Cow<'_, str>,
        req: &ProxiedHttpRequest,
        res: &ProxiedHttpResponse,
    ) -> bool;
}
