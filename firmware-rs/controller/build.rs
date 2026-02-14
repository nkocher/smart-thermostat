fn main() {
    if std::env::var_os("CARGO_FEATURE_ESP32").is_some() {
        embuild::espidf::sysenv::output();
    }
}
