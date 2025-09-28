use prometheus_client::encoding::EncodeLabelSet;

// LabelsProxy ---------------------------------------------------------

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct LabelsProxy {
    pub(crate) proxy: &'static str,
}

// LabelsProxyHttpJrpc -------------------------------------------------

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct LabelsProxyHttpJrpc {
    pub(crate) proxy: &'static str,

    pub(crate) jrpc_method: String,
}

// LabelsProxyWs -------------------------------------------------------

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct LabelsProxyWs {
    pub(crate) proxy: &'static str,
    pub(crate) destination: &'static str,
}
