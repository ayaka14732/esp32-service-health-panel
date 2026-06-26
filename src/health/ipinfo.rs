use embedded_svc::{
    http::{client::Client as HttpClient, Method},
    utils::io,
};
use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};

const HEALTH_URL: &str = "https://ipinfo.shn.hk/";

pub fn ok() -> bool {
    match check() {
        Ok(true) => true,
        Ok(false) => {
            log::error!("IP info health check failed");
            false
        }
        Err(err) => {
            log::error!("IP info health check error: {err:?}");
            false
        }
    }
}

fn check() -> Result<bool, Box<dyn std::error::Error>> {
    let http_config = HttpConfiguration {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(core::time::Duration::from_secs(10)),
        ..Default::default()
    };
    let mut client = HttpClient::wrap(EspHttpConnection::new(&http_config)?);

    let headers = [("accept", "text/plain")];
    let request = client.request(Method::Get, HEALTH_URL, &headers)?;
    log::info!("IP info health check: GET {HEALTH_URL}");
    let mut response = request.submit()?;

    let status = response.status();
    let mut buf = [0u8; 128];
    let bytes_read = io::try_read_full(&mut response, &mut buf).map_err(|e| e.0)?;
    let body = core::str::from_utf8(&buf[..bytes_read])?;
    // The endpoint returns the client's IP address on the first line (IPv4 or IPv6).
    let first_line = body.lines().next().unwrap_or("").trim();
    let first_line_is_ip = is_ip_address(first_line);

    log::info!("IP info health check response: status={status}, body={body:?}");

    Ok(status == 200 && first_line_is_ip)
}

/// Lightweight check: the response line should look like an IPv4 or IPv6 address.
fn is_ip_address(s: &str) -> bool {
    !s.is_empty()
        && (s.contains('.') || s.contains(':'))
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() || c == '.' || c == ':')
}
