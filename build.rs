// build.rs
// esp-idf-sys 需要這個來找 ESP-IDF toolchain
fn main() {
    embuild::espidf::sysenv::output();
}
