fn main() {
    let config = folio_service::ServiceConfig::from_env();
    if std::env::args().any(|argument| argument == "--healthcheck") {
        let address = config
            .bind
            .strip_prefix("0.0.0.0")
            .map(|rest| format!("127.0.0.1{rest}"))
            .unwrap_or_else(|| config.bind.clone());
        if let Err(error) = folio_service::healthcheck(&address) {
            eprintln!("folio-service healthcheck: {error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = folio_service::run(config) {
        eprintln!("folio-service: {error}");
        std::process::exit(1);
    }
}
