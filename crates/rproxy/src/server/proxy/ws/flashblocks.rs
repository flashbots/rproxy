use crate::server::proxy::{config::ConfigFlashblocks, ws::ProxyWsInner};
const PROXY_WS_FLASHBLOCKS_RPC_NAME: &str = "rproxy-flashblocks";

// ProxyWsInnerFlashblocks ---------------------------------------------

#[derive(Clone)]
pub(crate) struct ProxyWsInnerFlashblocks {
    config: ConfigFlashblocks,
}

impl ProxyWsInner<ConfigFlashblocks> for ProxyWsInnerFlashblocks {
    #[inline]
    fn name() -> &'static str {
        PROXY_WS_FLASHBLOCKS_RPC_NAME
    }

    fn new(config: ConfigFlashblocks) -> Self {
        Self { config }
    }

    fn config(&self) -> &ConfigFlashblocks {
        &self.config
    }
}
