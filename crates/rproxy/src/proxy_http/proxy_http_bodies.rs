use std::{
    fmt::Debug,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use actix_web::{self, http::StatusCode, web};
use awc::http::header::HeaderMap;
use bytes::Bytes;
use futures_core::Stream;
use pin_project::pin_project;
use time::UtcDateTime;
use tracing::warn;
use uuid::Uuid;

use crate::{
    config::ConfigProxyHttp,
    proxy_http::{
        ProxiedHttpRequest,
        ProxiedHttpResponse,
        ProxyHttp,
        ProxyHttpInner,
        ProxyHttpRequestInfo,
        ProxyHttpResponseInfo,
    },
};

// ProxyHttpRequestBody ------------------------------------------------

#[pin_project]
pub(crate) struct ProxyHttpRequestBody<S, C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    proxy: web::Data<ProxyHttp<C, P>>,

    info: Option<ProxyHttpRequestInfo>,
    start: UtcDateTime,
    body: Box<Vec<u8>>,

    #[pin]
    stream: S,
}

impl<S, C, P> ProxyHttpRequestBody<S, C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    pub(crate) fn new(
        worker: web::Data<ProxyHttp<C, P>>,
        info: ProxyHttpRequestInfo,
        body: S,
        timestamp: UtcDateTime,
    ) -> Self {
        Self {
            proxy: worker,
            info: Some(info),
            stream: body,
            start: timestamp,
            body: Box::new(Vec::new()),
        }
    }
}

impl<S, E, C, P> Stream for ProxyHttpRequestBody<S, C, P>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Debug,
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        match this.stream.poll_next(cx) {
            Poll::Pending => Poll::Pending,

            Poll::Ready(Some(Ok(chunk))) => {
                this.body.extend_from_slice(&chunk);
                Poll::Ready(Some(Ok(chunk)))
            }

            Poll::Ready(Some(Err(err))) => {
                if let Some(info) = mem::take(this.info) {
                    warn!(
                        proxy = P::name(),
                        request_id = %info.id(),
                        connection_id = %info.connection_id(),
                        error = ?err,
                        "Proxy http request stream error",
                    );
                } else {
                    warn!(
                        proxy = P::name(),
                        error = ?err,
                        request_id = "unknown",
                        "Proxy http request stream error",
                    );
                }
                Poll::Ready(Some(Err(err)))
            }

            Poll::Ready(None) => {
                let end = UtcDateTime::now();

                if let Some(info) = mem::take(this.info) {
                    let proxy = this.proxy.clone();

                    let req = ProxiedHttpRequest::new(
                        info,
                        mem::take(this.body),
                        this.start.clone(),
                        end,
                    );

                    proxy.postprocess_client_request(req);
                }

                Poll::Ready(None)
            }
        }
    }
}

// ProxyHttpResponseBody -----------------------------------------------

#[pin_project]
pub(crate) struct ProxyHttpResponseBody<S, C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    proxy: web::Data<ProxyHttp<C, P>>,

    info: Option<ProxyHttpResponseInfo>,
    start: UtcDateTime,
    body: Box<Vec<u8>>,

    #[pin]
    stream: S,
}

impl<S, C, P> ProxyHttpResponseBody<S, C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    pub(crate) fn new(
        proxy: web::Data<ProxyHttp<C, P>>,
        id: Uuid,
        status: StatusCode,
        headers: HeaderMap,
        body: S,
        timestamp: UtcDateTime,
    ) -> Self {
        Self {
            proxy,
            stream: body,
            start: timestamp,
            body: Box::new(Vec::new()),
            info: Some(ProxyHttpResponseInfo::new(id, status, headers)),
        }
    }
}

impl<S, E, C, P> Stream for ProxyHttpResponseBody<S, C, P>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Debug,
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        match this.stream.poll_next(cx) {
            Poll::Pending => Poll::Pending,

            Poll::Ready(Some(Ok(chunk))) => {
                this.body.extend_from_slice(&chunk);
                Poll::Ready(Some(Ok(chunk)))
            }

            Poll::Ready(Some(Err(err))) => {
                if let Some(info) = mem::take(this.info) {
                    warn!(
                        proxy = P::name(),
                        request_id = %info.id(),
                        error = ?err,
                        "Proxy http response stream error",
                    );
                } else {
                    warn!(
                        proxy = P::name(),
                        error = ?err,
                        request_id = "unknown",
                        "Proxy http response stream error",
                    );
                }
                Poll::Ready(Some(Err(err)))
            }

            Poll::Ready(None) => {
                let end = UtcDateTime::now();

                if let Some(info) = mem::take(this.info) {
                    let proxy = this.proxy.clone();

                    let res = ProxiedHttpResponse::new(
                        info,
                        mem::take(this.body),
                        this.start.clone(),
                        end,
                    );

                    proxy.postprocess_backend_response(res);
                }

                Poll::Ready(None)
            }
        }
    }
}
