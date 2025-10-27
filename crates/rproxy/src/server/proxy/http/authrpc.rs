use std::time::Duration;

use actix_http::header;
use actix_web::HttpResponse;
use moka::sync::Cache;

use crate::{
    jrpc::{JrpcRequestMeta, JrpcRequestMetaMaybeBatch},
    server::proxy::{
        config::ConfigAuthrpc,
        http::{ProxiedHttpRequest, ProxiedHttpResponse, ProxyHttpInner},
    },
};

const PROXY_HTTP_INNER_AUTHRPC_NAME: &str = "rproxy-authrpc";
const HASH_LEN: usize = 66; // 2 (for 0x) + 64 (for 32 bytes)

// ProxyHttpInnerAuthrpc -----------------------------------------------

#[derive(Clone)]
pub(crate) struct ProxyHttpInnerAuthrpc {
    config: ConfigAuthrpc,
    fcu_cache: Cache<[u8; 3 * HASH_LEN], ()>, // head + safe + finalised
}

impl ProxyHttpInner<ConfigAuthrpc> for ProxyHttpInnerAuthrpc {
    #[inline]
    fn name() -> &'static str {
        PROXY_HTTP_INNER_AUTHRPC_NAME
    }

    fn new(config: ConfigAuthrpc) -> Self {
        Self {
            config,
            fcu_cache: Cache::builder()
                .time_to_live(Duration::from_mins(1))
                .max_capacity(4096)
                .build(),
        }
    }

    #[inline]
    fn config(&self) -> &ConfigAuthrpc {
        &self.config
    }

    #[inline]
    fn might_intercept(&self) -> bool {
        self.config.deduplicate_fcus_wo_payload
    }

    fn should_mirror(
        &self,
        jrpc_req: &JrpcRequestMetaMaybeBatch,
        _: &ProxiedHttpRequest,
        _: &ProxiedHttpResponse,
    ) -> bool {
        fn should_mirror(jrpc_req: &JrpcRequestMeta) -> bool {
            let method = jrpc_req.method();

            if !method.starts_with("engine_forkchoiceUpdated") &&
                !method.starts_with("engine_newPayload") &&
                !method.starts_with("miner_setMaxDASize")
            {
                return false;
            }
            true
        }

        match jrpc_req {
            JrpcRequestMetaMaybeBatch::Single(jrpc_req_single) => should_mirror(jrpc_req_single),

            JrpcRequestMetaMaybeBatch::Batch(jrpc_req_batch) => {
                for jrpc_req_single in jrpc_req_batch.iter() {
                    if should_mirror(jrpc_req_single) {
                        return true;
                    }
                }
                false
            }
        }
    }

    fn should_intercept(
        &self,
        jrpc_req: &JrpcRequestMetaMaybeBatch,
    ) -> Option<Result<HttpResponse, actix_web::Error>> {
        let JrpcRequestMetaMaybeBatch::Single(jrpc_req) = jrpc_req else {
            return None;
        };

        if !jrpc_req.method_enriched().starts_with("engine_forkchoiceUpdated") ||
            jrpc_req.method_enriched().ends_with("withPayload")
        {
            return None;
        }

        let params = jrpc_req.params();

        if params.len() != 1 {
            return None;
        }

        let state = params[0].as_object()?;
        let head = state.get("headBlockHash")?.as_str()?;
        let safe = state.get("safeBlockHash")?.as_str()?;
        let finalized = state.get("finalizedBlockHash")?.as_str()?;

        let mut key = [0u8; 3 * HASH_LEN];
        key[0..HASH_LEN].copy_from_slice(head.as_bytes());
        key[HASH_LEN..2 * HASH_LEN].copy_from_slice(safe.as_bytes());
        key[2 * HASH_LEN..3 * HASH_LEN].copy_from_slice(finalized.as_bytes());

        if !self.fcu_cache.contains_key(&key) {
            self.fcu_cache.insert(key, ());
            return None;
        }

        let body = if jrpc_req.id().is_number() {
            format!(
                r#"{{"jsonrpc":"2.0","id":{},"result":{{"payloadStatus":{{"status":"VALID","latestValidHash":"{head}"}}}}}}"#,
                jrpc_req.id(),
            )
        } else if jrpc_req.id().is_string() {
            format!(
                r#"{{"jsonrpc":"2.0","id":"{}","result":{{"payloadStatus":{{"status":"VALID","latestValidHash":"{head}"}}}}}}"#,
                jrpc_req.id(),
            )
        } else {
            format!(
                r#"{{"jsonrpc":"2.0","result":{{"payloadStatus":{{"status":"VALID","latestValidHash":"{head}"}}}}}}"#,
            )
        };

        Some(Ok(HttpResponse::Ok()
            .append_header((header::CACHE_STATUS, "rproxy; hit"))
            .append_header((header::CONTENT_TYPE, "application/json; charset=utf-8"))
            .body(body)))
    }
}
