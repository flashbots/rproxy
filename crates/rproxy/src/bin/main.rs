use rproxy::{config::Config, server::Server};

// main ----------------------------------------------------------------

#[actix_web::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send>> {
    let cfg = Config::setup();

    Server::run(cfg).await
}
