use crate::server::proxy::{ProxyInner, ws::config::ConfigProxyWs};

// ProxyWsInner --------------------------------------------------------

pub(crate) trait ProxyWsInner<C>: ProxyInner + Clone + Send + Sync
where
    C: ConfigProxyWs,
{
    fn new(config: C) -> Self;
    fn config(&self) -> &C;
}
