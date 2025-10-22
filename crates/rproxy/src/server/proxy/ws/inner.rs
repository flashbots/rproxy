use crate::server::proxy::ws::config::ConfigProxyWs;

// ProxyWsInner --------------------------------------------------------

pub(crate) trait ProxyWsInner<C>: Clone + Send + Sync + 'static
where
    C: ConfigProxyWs,
{
    fn new(config: C) -> Self;
    fn config(&self) -> &C;
}
