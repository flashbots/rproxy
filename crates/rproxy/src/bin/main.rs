use rproxy::{config::Config, server::Server};

// main ----------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send>> {
    let cfg = Config::setup();

    Server::run(cfg).await
}
