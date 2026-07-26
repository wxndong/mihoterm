pub(crate) fn install_default_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
