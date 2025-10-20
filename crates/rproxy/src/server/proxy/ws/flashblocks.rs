use crate::server::proxy::{ProxyInner, config::ConfigFlashblocks, ws::ProxyWsInner};
const PROXY_WS_FLASHBLOCKS_RPC_NAME: &str = "rproxy-flashblocks";

// ProxyWsInnerFlashblocks ---------------------------------------------

#[derive(Clone)]
pub(crate) struct ProxyWsInnerFlashblocks {
    config: ConfigFlashblocks,
}

impl ProxyInner for ProxyWsInnerFlashblocks {
    fn name() -> &'static str {
        PROXY_WS_FLASHBLOCKS_RPC_NAME
    }
}

impl ProxyWsInner<ConfigFlashblocks> for ProxyWsInnerFlashblocks {
    fn new(config: ConfigFlashblocks) -> Self {
        Self { config }
    }

    fn config(&self) -> &ConfigFlashblocks {
        &self.config
    }
}
