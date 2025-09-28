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
  -h, --help     Print help
  -V, --version  Print version

authrpc:
      --authrpc-backend <url>
          url of authrpc backend [env: RPROXY_AUTHRPC_BACKEND=] [default: http://127.0.0.1:18651]
      --authrpc-backend-max-concurrent-requests <count>
          max concurrent requests per authrpc backend [env:
          RPROXY_AUTHRPC_BACKEND_MAX_CONCURRENT_REQUESTS=] [default: 10]
      --authrpc-backend-timeout <duration>
          max duration for authrpc backend requests [env: RPROXY_AUTHRPC_BACKEND_TIMEOUT=] [default:
          30s]
      --authrpc-enabled
          enable authrpc proxy [env: RPROXY_AUTHRPC_ENABLED=]
      --authrpc-idle-connection-timeout <duration>
          duration to keep idle authrpc connections open (0 means no keep-alive) [env:
          RPROXY_AUTHRPC_IDLE_CONNECTION_TIMEOUT=] [default: 30s]
      --authrpc-listen-address <socket>
          host:port for authrpc proxy [env: RPROXY_AUTHRPC_LISTEN_ADDRESS=] [default: 0.0.0.0:8651]
      --authrpc-log-mirrored-requests
          whether to log proxied authrpc requests [env: RPROXY_AUTHRPC_LOG_MIRRORED_REQUESTS=]
      --authrpc-log-mirrored-responses
          whether to log responses to proxied authrpc requests [env:
          RPROXY_AUTHRPC_LOG_MIRRORED_RESPONSES=]
      --authrpc-log-proxied-requests
          whether to log proxied authrpc requests [env: RPROXY_AUTHRPC_LOG_PROXIED_REQUESTS=]
      --authrpc-log-proxied-responses
          whether to log responses to proxied authrpc requests [env:
          RPROXY_AUTHRPC_LOG_PROXIED_RESPONSES=]
      --authrpc-log-sanitise
          sanitise logs of proxied authrpc requests/responses (e.g. don't log raw transactions)
          [env: RPROXY_AUTHRPC_LOG_SANITISE=]
      --authrpc-peer <url>...
          list of authrpc peers urls to mirror the requests to [env: RPROXY_AUTHRPC_PEERS=]
      --authrpc-remove-backend-from-peers
          remove authrpc backend from peers [env: RPROXY_AUTHRPC_REMOVE_BACKEND_FROM_PEERS=]

circuit-breaker:
      --circuit-breaker-reset-continuously
          reset proxies continuously at each poll interval as long as circuit-breaker url reports
          unhealthy [env: RPROXY_CIRCUIT_BREAKER_RESET_CONTINUOUSLY=]
      --circuit-breaker-poll-interval <duration>
          circuit breaker's poll interval [env: RPROXY_CIRCUIT_BREAKER_POLL_INTERVAL=] [default: 5s]
      --circuit-breaker-threshold-healthy <count>
          healthy threshold for circuit-breaker [env: RPROXY_CIRCUIT_BREAKER_THRESHOLD_HEALTHY=]
          [default: 2]
      --circuit-breaker-threshold-unhealthy <count>
          unhealthy threshold for circuit-breaker [env: RPROXY_CIRCUIT_BREAKER_THRESHOLD_UNHEALTHY=]
          [default: 3]
      --circuit-breaker-url <url>
          url of circuit-breaker (e.g. backend healthcheck) [env: RPROXY_CIRCUIT_BREAKER_URL=]
          [default: ]

flashblocks:
      --flashblocks-backend <url>
          url of flashblocks backend [env: RPROXY_FLASHBLOCKS_BACKEND=] [default:
          ws://127.0.0.1:11111]
      --flashblocks-enabled
          enable flashblocks proxy [env: RPROXY_FLASHBLOCKS_ENABLED=]
      --flashblocks-backend-timeout <duration>
          timeout to establish backend connections of to receive pong websocket response [env:
          RPROXY_FLASHBLOCKS_BACKEND_TIMEOUT=] [default: 30s]
      --flashblocks-listen-address <socket>
          host:port for flashblocks proxy [env: RPROXY_FLASHBLOCKS_LISTEN_ADDRESS=] [default:
          0.0.0.0:1111]
      --flashblocks-log-backend-messages
          whether to log flashblocks backend messages [env:
          RPROXY_FLASHBLOCKS_LOG_BACKEND_MESSAGES=]
      --flashblocks-log-client-messages
          whether to log flashblocks backend messages whether to log flashblocks client messages
          [env: RPROXY_FLASHBLOCKS_LOG_CLIENT_MESSAGES=]
      --flashblocks-log-sanitise
          sanitise logs of proxied flashblocks messages (e.g. don't log raw transactions) [env:
          RPROXY_FLASHBLOCKS_LOG_SANITISE=]

log:
      --log-format <format>  logging format [env: RPROXY_LOG_FORMAT=] [default: json] [possible
                             values: json, text]
      --log-level <level>    logging level [env: RPROXY_LOG_LEVEL=] [default: info]

metrics:
      --metrics-listen-address <socket>
          host:port for metrics [env: RPROXY_METRICS_LISTEN_ADDRESS=] [default: 0.0.0.0:6785]

rpc:
      --rpc-backend <url>
          url of rpc backend [env: RPROXY_RPC_BACKEND=] [default: http://127.0.0.1:18645]
      --rpc-backend-max-concurrent-requests <count>
          max concurrent requests per backend [env: RPROXY_RPC_BACKEND_MAX_CONCURRENT_REQUESTS=]
          [default: 10]
      --rpc-backend-timeout <duration>
          max duration for backend requests [env: RPROXY_RPC_BACKEND_TIMEOUT=] [default: 30s]
      --rpc-enabled
          enable rpc proxy [env: RPROXY_RPC_ENABLED=]
      --rpc-idle-connection-timeout <duration>
          duration to keep idle rpc connections open (0 means no keep-alive) [env:
          RPROXY_RPC_IDLE_CONNECTION_TIMEOUT=] [default: 30s]
      --rpc-listen-address <socket>
          host:port for rpc proxy [env: RPROXY_RPC_LISTEN_ADDRESS=] [default: 0.0.0.0:8645]
      --rpc-log-mirrored-requests
          whether to log proxied rpc requests [env: RPROXY_RPC_LOG_MIRRORED_REQUESTS=]
      --rpc-log-mirrored-responses
          whether to log responses to proxied rpc requests [env: RPROXY_RPC_LOG_MIRRORED_RESPONSES=]
      --rpc-log-proxied-requests
          whether to log proxied rpc requests [env: RPROXY_RPC_LOG_PROXIED_REQUESTS=]
      --rpc-log-proxied-responses
          whether to log responses to proxied rpc requests [env: RPROXY_RPC_LOG_PROXIED_RESPONSES=]
      --rpc-log-sanitise
          sanitise logs of proxied rpc requests/responses (e.g. don't log raw transactions) [env:
          RPROXY_RPC_LOG_SANITISE=]
      --rpc-mirror-errored-requests
          whether the requests that returned an error from rpc backend should be mirrored to peers
          [env: RPROXY_RPC_MIRROR_ERRORED_REQUESTS=]
      --rpc-peer <url>...
          list of rpc peers urls to mirror the requests to [env: RPROXY_RPC_PEERS=]
      --rpc-remove-backend-from-peers
          remove rpc backend from peers [env: RPROXY_RPC_REMOVE_BACKEND_FROM_PEERS=]

tls:
      --tls-certificate <path>  path to tls certificate [env: RPROXY_TLS_CERTIFICATE=] [default: ]
      --tls-key <path>          path to tls key [env: RPROXY_TLS_KEY=] [default: ]
```
