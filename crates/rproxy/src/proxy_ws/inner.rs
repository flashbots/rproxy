use crate::{config::ConfigProxyWs, proxy::ProxyInner};

// ProxyWsInner --------------------------------------------------------

pub(crate) trait ProxyWsInner<C>: ProxyInner + Clone + Send + Sync
where
    C: ConfigProxyWs,
{
    fn new(config: C) -> Self;
    fn config(&self) -> &C;
}
