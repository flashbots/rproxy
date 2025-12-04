# `rproxy`

>
> [!IMPORTANT]
>
> This project is WORK-IN-PROGRESS.  Not for production use (yet)
>

L2 builder proxy that:

- Proxies RPC and Authenticated RPC calls to the builder.

- Mirrors the following calls to other builders (a.k.a. peers):

  - `engine_forkchoiceUpdated`
  - `engine_newPayload`
  - `eth_sendBundle`
  - `eth_sendRawTransaction`
  - `miner_setMaxDASize`

- Proxies flashblocks stream of the builder.

- Allows circuit-breaking:

  - If the healthcheck fails for certain count of times, `rproxy` will
    gracefully terminate all proxied connections (helps if the builders are L4
    load-balanced).

- Enables TLS termination.

- Allows debug-logging of all proxied messages and/or responses.

  - In production scenarios it allows to sanitise the logs by replacing raw
    transactions with their respective hashes.

- Enables observability with prometheus metrics for request counts, sizes,
  latencies, and so on.

## Usage

```text
Usage: rproxy [OPTIONS]

Options:
  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

authrpc:
      --authrpc-backend <url>
          url of authrpc backend

          [env: RPROXY_AUTHRPC_BACKEND=]
          [default: http://127.0.0.1:18651]

      --authrpc-backend-max-concurrent-requests <count>
          max concurrent requests per authrpc backend

          [env: RPROXY_AUTHRPC_BACKEND_MAX_CONCURRENT_REQUESTS=]
          [default: 1]

      --authrpc-backend-timeout <duration>
          max duration for authrpc backend requests

          [env: RPROXY_AUTHRPC_BACKEND_TIMEOUT=]
          [default: 30s]

      --authrpc-deduplicate-fcus-wo-payload
          whether authrpc proxy should deduplicate incoming fcus w/o payload (mitigates
          fcu avalanche issue)

          [env: RPROXY_AUTHRPC_DEDUPLICATE_FCUS_WO_PAYLOAD=]

      --authrpc-enabled
          enable authrpc proxy

          [env: RPROXY_AUTHRPC_ENABLED=]

      --authrpc-idle-connection-timeout <duration>
          duration to keep idle authrpc connections open (0 means no keep-alive)

          [env: RPROXY_AUTHRPC_IDLE_CONNECTION_TIMEOUT=]
          [default: 30s]

      --authrpc-keepalive-interval <duration>
          interval between tcp keepalive packets on authrpc connections

          [env: RPROXY_AUTHRPC_KEEPALIVE_INTERVAL=]
          [default: 5s]

      --authrpc-listen-address <socket>
          host:port for authrpc proxy

          [env: RPROXY_AUTHRPC_LISTEN_ADDRESS=]
          [default: 0.0.0.0:8651]

      --authrpc-log-mirrored-requests
          whether to log proxied authrpc requests

          [env: RPROXY_AUTHRPC_LOG_MIRRORED_REQUESTS=]

      --authrpc-log-mirrored-responses
          whether to log responses to proxied authrpc requests

          [env: RPROXY_AUTHRPC_LOG_MIRRORED_RESPONSES=]

      --authrpc-log-proxied-requests
          whether to log proxied authrpc requests

          [env: RPROXY_AUTHRPC_LOG_PROXIED_REQUESTS=]

      --authrpc-log-proxied-responses
          whether to log responses to proxied authrpc requests

          [env: RPROXY_AUTHRPC_LOG_PROXIED_RESPONSES=]

      --authrpc-log-sanitise
          sanitise logs of proxied authrpc requests/responses (e.g. don't log raw
          transactions)

          [env: RPROXY_AUTHRPC_LOG_SANITISE=]

      --authrpc-max-request-size-mb <megabytes>
          max size of authrpc requests

          [env: RPROXY_AUTHRPC_MAX_REQUEST_SIZE_MB=]
          [default: 16]

      --authrpc-max-response-size-mb <megabytes>
          max size of authrpc responses

          [env: RPROXY_AUTHRPC_MAX_RESPONSE_SIZE_MB=]
          [default: 256]

      --authrpc-mirroring-peer <url>...
          list of authrpc peers urls to mirror the requests to

          [env: RPROXY_AUTHRPC_MIRRORING_PEERS=]

      --authrpc-mirroring-strategy <strategy>
          Possible values:
          - fan-out:           mirror to all configured peers
          - round-robin:       mirror to only 1 peer at a time, in round-robin fashion
          - round-robin-pairs: mirror to 2 peers at a time, in round-robin fashion

          [env: RPROXY_AUTHRPC_MIRRORING_STRATEGY=]
          [default: fan-out]

      --authrpc-preallocated-request-buffer-size-kb <kilobytes>
          size of preallocated authrpc request buffers

          [env: RPROXY_AUTHRPC_PREALLOCATED_RESPONSE_BUFFER_SIZE_KB=]
          [default: 1]

      --authrpc-preallocated-response-buffer-size-kb <kilobytes>
          size of preallocated authrpc response buffers

          [env: RPROXY_AUTHRPC_PREALLOCATED_RESPONSE_BUFFER_SIZE_KB=]
          [default: 1]

      --authrpc-remove-backend-from-mirroring-peers
          remove authrpc backend from mirroring peers

          [env: RPROXY_AUTHRPC_REMOVE_BACKEND_FROM_MIRRORING_PEERS=]

circuit-breaker:
      --circuit-breaker-reset-continuously
          reset proxies continuously at each poll interval as long as circuit-breaker url
          reports unhealthy

          [env: RPROXY_CIRCUIT_BREAKER_RESET_CONTINUOUSLY=]

      --circuit-breaker-poll-interval <duration>
          circuit breaker's poll interval

          [env: RPROXY_CIRCUIT_BREAKER_POLL_INTERVAL=]
          [default: 5s]

      --circuit-breaker-threshold-healthy <count>
          healthy threshold for circuit-breaker

          [env: RPROXY_CIRCUIT_BREAKER_THRESHOLD_HEALTHY=]
          [default: 2]

      --circuit-breaker-threshold-unhealthy <count>
          unhealthy threshold for circuit-breaker

          [env: RPROXY_CIRCUIT_BREAKER_THRESHOLD_UNHEALTHY=]
          [default: 3]

      --circuit-breaker-url <url>
          url of circuit-breaker (e.g. backend healthcheck)

          [env: RPROXY_CIRCUIT_BREAKER_URL=]
          [default: ]

flashblocks:
      --flashblocks-backend-timeout <duration>
          timeout to establish backend connections of to receive pong websocket response

          [env: RPROXY_FLASHBLOCKS_BACKEND_TIMEOUT=]
          [default: 30s]

      --flashblocks-backend <url>
          url of flashblocks backend

          [env: RPROXY_FLASHBLOCKS_BACKEND=]
          [default: ws://127.0.0.1:11111]

      --flashblocks-enabled
          enable flashblocks proxy

          [env: RPROXY_FLASHBLOCKS_ENABLED=]

      --flashblocks-keepalive-interval <duration>
          interval between tcp keepalive packets on flashblocks connections

          [env: RPROXY_FLASHBLOCKS_KEEPALIVE_INTERVAL=]
          [default: 5s]

      --flashblocks-listen-address <socket>
          host:port for flashblocks proxy

          [env: RPROXY_FLASHBLOCKS_LISTEN_ADDRESS=]
          [default: 0.0.0.0:1111]

      --flashblocks-log-backend-messages
          whether to log flashblocks backend messages

          [env: RPROXY_FLASHBLOCKS_LOG_BACKEND_MESSAGES=]

      --flashblocks-log-client-messages
          whether to log flashblocks client messages

          [env: RPROXY_FLASHBLOCKS_LOG_CLIENT_MESSAGES=]

      --flashblocks-log-sanitise
          sanitise logs of proxied flashblocks messages (e.g. don't log raw transactions)

          [env: RPROXY_FLASHBLOCKS_LOG_SANITISE=]

log:
      --log-format <format>
          logging format

          [env: RPROXY_LOG_FORMAT=]
          [default: json]
          [possible values: json, text]

      --log-level <level>
          logging level

          [env: RPROXY_LOG_LEVEL=]
          [default: info]

metrics:
      --metrics-listen-address <socket>
          host:port for metrics

          [env: RPROXY_METRICS_LISTEN_ADDRESS=]
          [default: 0.0.0.0:6785]

rpc:
      --rpc-backend <url>
          url of rpc backend

          [env: RPROXY_RPC_BACKEND=]
          [default: http://127.0.0.1:18645]

      --rpc-backend-max-concurrent-requests <count>
          max concurrent requests per backend

          [env: RPROXY_RPC_BACKEND_MAX_CONCURRENT_REQUESTS=]
          [default: 10]

      --rpc-backend-timeout <duration>
          max duration for backend requests

          [env: RPROXY_RPC_BACKEND_TIMEOUT=]
          [default: 30s]

      --rpc-enabled
          enable rpc proxy

          [env: RPROXY_RPC_ENABLED=]

      --rpc-idle-connection-timeout <duration>
          duration to keep idle rpc connections open (0 means no keep-alive)

          [env: RPROXY_RPC_IDLE_CONNECTION_TIMEOUT=]
          [default: 30s]

      --rpc-keepalive-interval <duration>
          interval between tcp keepalive packets on rpc connections

          [env: RPROXY_RPC_KEEPALIVE_INTERVAL=]
          [default: 5s]

      --rpc-listen-address <socket>
          host:port for rpc proxy

          [env: RPROXY_RPC_LISTEN_ADDRESS=]
          [default: 0.0.0.0:8645]

      --rpc-log-mirrored-requests
          whether to log proxied rpc requests

          [env: RPROXY_RPC_LOG_MIRRORED_REQUESTS=]

      --rpc-log-mirrored-responses
          whether to log responses to proxied rpc requests

          [env: RPROXY_RPC_LOG_MIRRORED_RESPONSES=]

      --rpc-log-proxied-requests
          whether to log proxied rpc requests

          [env: RPROXY_RPC_LOG_PROXIED_REQUESTS=]

      --rpc-log-proxied-responses
          whether to log responses to proxied rpc requests

          [env: RPROXY_RPC_LOG_PROXIED_RESPONSES=]

      --rpc-log-sanitise
          sanitise logs of proxied rpc requests/responses (e.g. don't log raw
          transactions)

          [env: RPROXY_RPC_LOG_SANITISE=]

      --rpc-max-request-size-mb <megabytes>
          max size of rpc requests

          [env: RPROXY_RPC_MAX_REQUEST_SIZE_MB=]
          [default: 16]

      --rpc-max-response-size-mb <megabytes>
          max size of rpc responses

          [env: RPROXY_RPC_MAX_RESPONSE_SIZE_MB=]
          [default: 256]

      --rpc-mirror-errored-requests
          whether the requests that returned an error from rpc backend should be mirrored
          to peers

          [env: RPROXY_RPC_MIRROR_ERRORED_REQUESTS=]

      --rpc-mirroring-peer <url>...
          list of rpc peers urls to mirror the requests to

          [env: RPROXY_RPC_MIRRORING_PEERS=]

      --rpc-mirroring-strategy <strategy>
          Possible values:
          - fan-out:           mirror to all configured peers
          - round-robin:       mirror to only 1 peer at a time, in round-robin fashion
          - round-robin-pairs: mirror to 2 peers at a time, in round-robin fashion

          [env: RPROXY_RPC_MIRRORING_STRATEGY=]
          [default: fan-out]

      --rpc-preallocated-request-buffer-size-kb <kilobytes>
          size of preallocated rpc request buffers

          [env: RPROXY_RPC_PREALLOCATED_RESPONSE_BUFFER_SIZE_KB=]
          [default: 1]

      --rpc-preallocated-response-buffer-size-kb <kilobytes>
          size of preallocated rpc response buffers

          [env: RPROXY_RPC_PREALLOCATED_RESPONSE_BUFFER_SIZE_KB=]
          [default: 256]

      --rpc-remove-backend-from-mirroring-peers
          remove rpc backend from peers

          [env: RPROXY_RPC_REMOVE_BACKEND_FROM_MIRRORING_PEERS=]

tls:
      --tls-certificate <path>
          path to tls certificate

          [env: RPROXY_TLS_CERTIFICATE=]
          [default: ]

      --tls-key <path>
          path to tls key

          [env: RPROXY_TLS_KEY=]
          [default: ]
```

### Chaos

These flags are enabled when `rproxy` is built with `chaos` feature enabled:

```text
chaos:
      --chaos-probability-flashblocks-backend-ping-ignored <probability>
          the chance (between 0.0 and 1.0) that pings received from flashblocks backend
          would be ignored (no pong sent)

          [env: RPROXY_CHAOS_PROBABILITY_FLASHBLOCKS_BACKEND_PING_IGNORED=]
          [default: 0.0]

      --chaos-probability-flashblocks-client-ping-ignored <probability>
          the chance (between 0.0 and 1.0) that pings received from flashblocks client
          would be ignored (no pong sent)

          [env: RPROXY_CHAOS_PROBABILITY_FLASHBLOCKS_CLIENT_PING_IGNORED=]
          [default: 0.0]

      --chaos-probability-flashblocks-stream-blocked <probability>
          the chance (between 0.0 and 1.0) that client's flashblocks stream would block
          (no more messages sent)

          [env: RPROXY_CHAOS_PROBABILITY_FLASHBLOCKS_STREAM_BLOCKED=]
          [default: 0.0]
```

## Metrics

```prometheus
# HELP rproxy_client_connections_active_count count of active client connections.
# TYPE rproxy_client_connections_active_count gauge

# HELP rproxy_client_connections_established_count count of client connections established.
# TYPE rproxy_client_connections_established_count counter

# HELP rproxy_client_info general information about the client.
# TYPE rproxy_client_info counter

# HELP rproxy_client_connections_closed_count count of client connections closed.
# TYPE rproxy_client_connections_closed_count counter

# HELP rproxy_http_latency_backend_nanoseconds latency of backend http responses (interval b/w end of client's request and begin of backend's response).
# TYPE rproxy_http_latency_backend_nanoseconds gauge
# UNIT rproxy_http_latency_backend_nanoseconds nanoseconds

# HELP rproxy_http_latency_delta_nanoseconds latency delta (http_latency_total - http_latency_backend).
# TYPE rproxy_http_latency_delta_nanoseconds gauge
# UNIT rproxy_http_latency_delta_nanoseconds nanoseconds

# HELP rproxy_http_latency_total_nanoseconds overall latency of http requests (interval b/w begin of client's request and end of forwarded response).
# TYPE rproxy_http_latency_total_nanoseconds gauge
# UNIT rproxy_http_latency_total_nanoseconds nanoseconds

# HELP rproxy_http_mirror_success_count count of successfully mirrored http requests/responses.
# TYPE rproxy_http_mirror_success_count counter

# HELP rproxy_http_mirror_failure_count count of failures to mirror http request/response.
# TYPE rproxy_http_mirror_failure_count counter

# HELP rproxy_http_proxy_success_count count of successfully proxied http requests/responses.
# TYPE rproxy_http_proxy_success_count counter

# HELP rproxy_http_proxy_failure_count count of failures to proxy http request/response.
# TYPE rproxy_http_proxy_failure_count counter

# HELP rproxy_http_request_size_bytes sizes of incoming http requests.
# TYPE rproxy_http_request_size_bytes gauge
# UNIT rproxy_http_request_size_bytes bytes

# HELP rproxy_http_response_size_bytes sizes of proxied http responses.
# TYPE rproxy_http_response_size_bytes gauge
# UNIT rproxy_http_response_size_bytes bytes

# HELP rproxy_http_request_decompressed_size_bytes decompressed sizes of incoming http requests.
# TYPE rproxy_http_request_decompressed_size_bytes gauge
# UNIT rproxy_http_request_decompressed_size_bytes bytes

# HELP rproxy_http_response_decompressed_size_bytes decompressed sizes of proxied http responses.
# TYPE rproxy_http_response_decompressed_size_bytes gauge
# UNIT rproxy_http_response_decompressed_size_bytes bytes

# HELP rproxy_tls_certificate_valid_not_before tls certificate's not-valid-before timestamp.
# TYPE rproxy_tls_certificate_valid_not_before gauge

# HELP rproxy_tls_certificate_valid_not_after tls certificate's not-valid-after timestamp.
# TYPE rproxy_tls_certificate_valid_not_after gauge

# HELP rproxy_ws_latency_backend_nanoseconds round-trip-time of websocket pings to backend divided by 2.
# TYPE rproxy_ws_latency_backend_nanoseconds gauge
# UNIT rproxy_ws_latency_backend_nanoseconds nanoseconds

# HELP rproxy_ws_latency_client_nanoseconds round-trip-time of websocket pings to client divided by 2.
# TYPE rproxy_ws_latency_client_nanoseconds gauge
# UNIT rproxy_ws_latency_client_nanoseconds nanoseconds

# HELP rproxy_ws_latency_proxy_nanoseconds time to process the websocket message by the proxy.
# TYPE rproxy_ws_latency_proxy_nanoseconds gauge
# UNIT rproxy_ws_latency_proxy_nanoseconds nanoseconds

# HELP rproxy_ws_message_size_bytes sizes of proxied websocket messages.
# TYPE rproxy_ws_message_size_bytes gauge
# UNIT rproxy_ws_message_size_bytes bytes

# HELP rproxy_ws_proxy_success_count count of successfully proxied websocket messages.
# TYPE rproxy_ws_proxy_success_count counter

# HELP rproxy_ws_proxy_failure_count count of failures to proxy websocket message.
# TYPE rproxy_ws_proxy_failure_count counter
```
