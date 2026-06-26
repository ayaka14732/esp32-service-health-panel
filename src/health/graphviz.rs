use embedded_svc::{
    http::{client::Client as HttpClient, Method},
    io::Write,
    utils::io,
};
use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};

const HEALTH_URL: &str = "https://backboard.railway.com/graphql/v2";
const RAILWAY_TOKEN: Option<&str> = option_env!("RAILWAY_TOKEN");
const RAILWAY_PROJECT_ID: Option<&str> = option_env!("RAILWAY_PROJECT_ID");
const RAILWAY_SERVICE_ID: Option<&str> = option_env!("RAILWAY_SERVICE_ID");
const RAILWAY_ENVIRONMENT_ID: Option<&str> = option_env!("RAILWAY_ENVIRONMENT_ID");

pub fn ok() -> bool {
    match check() {
        Ok(true) => true,
        Ok(false) => {
            log::error!("Graphviz health check failed");
            false
        }
        Err(err) => {
            log::error!("Graphviz health check error: {err:?}");
            false
        }
    }
}

fn check() -> Result<bool, Box<dyn std::error::Error>> {
    let token = RAILWAY_TOKEN.ok_or("RAILWAY_TOKEN not set")?;
    let project_id = RAILWAY_PROJECT_ID.ok_or("RAILWAY_PROJECT_ID not set")?;
    let service_id = RAILWAY_SERVICE_ID.ok_or("RAILWAY_SERVICE_ID not set")?;
    let environment_id = RAILWAY_ENVIRONMENT_ID.ok_or("RAILWAY_ENVIRONMENT_ID not set")?;

    let body = format!(
        r#"{{"query":"query($projectId: String!, $serviceId: String!, $environmentId: String!) {{ deployments(input: {{ projectId: $projectId, serviceId: $serviceId, environmentId: $environmentId }}, first: 1) {{ edges {{ node {{ id status }} }} }} }}","variables":{{"projectId":"{project_id}","serviceId":"{service_id}","environmentId":"{environment_id}"}}}}"#
    );

    let http_config = HttpConfiguration {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(core::time::Duration::from_secs(10)),
        ..Default::default()
    };
    let mut client = HttpClient::wrap(EspHttpConnection::new(&http_config)?);

    let auth = format!("Bearer {token}");
    let headers = [
        ("authorization", auth.as_str()),
        ("content-type", "application/json"),
    ];
    let mut request = client.request(Method::Post, HEALTH_URL, &headers)?;
    log::info!("Graphviz health check: POST {HEALTH_URL}");
    request.write_all(body.as_bytes())?;
    let mut response = request.submit()?;

    let status = response.status();
    let mut buf = [0u8; 512];
    let bytes_read = io::try_read_full(&mut response, &mut buf).map_err(|e| e.0)?;
    let body = core::str::from_utf8(&buf[..bytes_read])?.trim();

    log::info!("Graphviz health check response: status={status}, body={body:?}");

    if status != 200 {
        return Ok(false);
    }

    let deployment_status = extract_status(body).unwrap_or("");
    Ok(matches!(deployment_status, "SUCCESS" | "SLEEPING"))
}

fn extract_status(body: &str) -> Option<&str> {
    let key = r#""status":""#;
    let start = body.find(key)? + key.len();
    let end = body[start..].find('"')?;
    Some(&body[start..start + end])
}
