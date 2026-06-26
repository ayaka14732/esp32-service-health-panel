use embedded_svc::{
    http::{client::Client as HttpClient, Method},
    utils::io,
};
use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};

const HEALTH_URL: &str = "https://uk-railway-journey-recorder-api.shn.hk/api/health";
const HEALTH_OK_BODY: &str = r#"{"status":"ok"}"#;

pub fn ok() -> bool {
    match check() {
        Ok(true) => true,
        Ok(false) => {
            log::error!("Railway API health check failed");
            false
        }
        Err(err) => {
            log::error!("Railway API health check error: {err:?}");
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

    let headers = [("accept", "application/json")];
    let request = client.request(Method::Get, HEALTH_URL, &headers)?;
    log::info!("Railway API health check: GET {HEALTH_URL}");
    let mut response = request.submit()?;

    let status = response.status();
    let mut buf = [0u8; 128];
    let bytes_read = io::try_read_full(&mut response, &mut buf).map_err(|e| e.0)?;
    let body = core::str::from_utf8(&buf[..bytes_read])?.trim();

    log::info!("Railway API health check response: status={status}, body={body:?}");

    Ok(status == 200 && body == HEALTH_OK_BODY)
}
