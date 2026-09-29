pub const CONTRACT: &str = "k8s-web-api-data-plane/v1";

pub fn modes() -> [&'static str; 4] {
    ["db-read", "http", "mtls-tcp", "jetstream"]
}
