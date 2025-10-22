use crate::server::proxy::{config::ConfigFlashblocks, ws::ProxyWsInner};

// ProxyWsInnerFlashblocks ---------------------------------------------

#[derive(Clone)]
pub(crate) struct ProxyWsInnerFlashblocks {
    config: ConfigFlashblocks,
}

impl ProxyWsInner<ConfigFlashblocks> for ProxyWsInnerFlashblocks {
    fn new(config: ConfigFlashblocks) -> Self {
        Self { config }
    }

    fn config(&self) -> &ConfigFlashblocks {
        &self.config
    }
}
