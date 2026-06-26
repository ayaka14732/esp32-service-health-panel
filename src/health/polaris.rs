use embedded_svc::http::{client::Client as HttpClient, Method};
use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};

const HEALTH_URL: &str = "https://syv.red/zh-TW";

pub fn ok() -> bool {
    match check() {
        Ok(true) => true,
        Ok(false) => {
            log::error!("Polaris health check failed");
            false
        }
        Err(err) => {
            log::error!("Polaris health check error: {err:?}");
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

    let headers = [("accept", "text/html")];
    let request = client.request(Method::Get, HEALTH_URL, &headers)?;
    log::info!("Polaris health check: GET {HEALTH_URL}");
    let response = request.submit()?;

    let status = response.status();
    log::info!("Polaris health check response: status={status}");

    Ok(status == 200)
}
