// build.rs
// esp-idf-sys 需要這個來找 ESP-IDF toolchain
use std::fs;

fn main() {
    println!("cargo:rerun-if-env-changed=WIFI_SSID");
    println!("cargo:rerun-if-env-changed=WIFI_PASS");
    println!("cargo:rerun-if-env-changed=RAILWAY_TOKEN");
    println!("cargo:rerun-if-env-changed=RAILWAY_PROJECT_ID");
    println!("cargo:rerun-if-env-changed=RAILWAY_SERVICE_ID");
    println!("cargo:rerun-if-env-changed=RAILWAY_ENVIRONMENT_ID");
    println!("cargo:rerun-if-changed=.env");

    if let Some(ssid) = env_value("WIFI_SSID") {
        println!("cargo:rustc-env=WIFI_SSID={ssid}");
    }
    if let Some(pass) = env_value("WIFI_PASS") {
        println!("cargo:rustc-env=WIFI_PASS={pass}");
    }
    if let Some(token) = env_value("RAILWAY_TOKEN") {
        println!("cargo:rustc-env=RAILWAY_TOKEN={token}");
    }
    if let Some(id) = env_value("RAILWAY_PROJECT_ID") {
        println!("cargo:rustc-env=RAILWAY_PROJECT_ID={id}");
    }
    if let Some(id) = env_value("RAILWAY_SERVICE_ID") {
        println!("cargo:rustc-env=RAILWAY_SERVICE_ID={id}");
    }
    if let Some(id) = env_value("RAILWAY_ENVIRONMENT_ID") {
        println!("cargo:rustc-env=RAILWAY_ENVIRONMENT_ID={id}");
    }

    embuild::espidf::sysenv::output();
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .or_else(|| dotenv_value(".env", key))
}

fn dotenv_value(path: &str, key: &str) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };

        if name.trim() == key {
            return Some(unquote(value.trim()));
        }
    }

    None
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let quote = bytes[0];
        if (quote == b'\'' || quote == b'"') && bytes[value.len() - 1] == quote {
            return value[1..value.len() - 1].to_string();
        }
    }

    value.to_string()
}
