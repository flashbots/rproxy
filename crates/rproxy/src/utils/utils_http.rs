// is_hop_by_hop_header ------------------------------------------------

pub fn is_hop_by_hop_header(name: &actix_web::http::header::HeaderName) -> bool {
    matches!(name.as_str().to_ascii_lowercase().as_str(), "connection" | "host" | "keep-alive")
}
