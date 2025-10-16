use std::borrow::Cow;

use prometheus_client::encoding::EncodeLabelSet;

// LabelsProxy ---------------------------------------------------------

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct LabelsProxy {
    pub(crate) proxy: &'static str,
}

// LabelsProxyClientInfo -----------------------------------------------

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct LabelsProxyClientInfo {
    pub(crate) proxy: &'static str,
    pub(crate) user_agent: String,
}

// LabelsProxyHttpJrpc -------------------------------------------------

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct LabelsProxyHttpJrpc {
    pub(crate) proxy: &'static str,
    pub(crate) jrpc_method: Cow<'static, str>,
}

// LabelsProxyWs -------------------------------------------------------

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct LabelsProxyWs {
    pub(crate) proxy: &'static str,
    pub(crate) destination: &'static str,
}
