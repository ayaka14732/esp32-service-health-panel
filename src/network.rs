use esp_idf_hal::{delay::FreeRtos, modem::Modem};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::EspDefaultNvsPartition,
    sntp::{EspSntp, SyncStatus},
    wifi::{
        AuthMethod, BlockingWifi, ClientConfiguration, Configuration as WifiConfiguration, EspWifi,
    },
};
use esp_idf_sys::EspError;

const WIFI_SSID: Option<&str> = option_env!("WIFI_SSID");
const WIFI_PASS: Option<&str> = option_env!("WIFI_PASS");

pub struct Network {
    wifi: Option<BlockingWifi<EspWifi<'static>>>,
    _sntp: Option<EspSntp<'static>>,
}

impl Network {
    pub fn is_connected(&self) -> bool {
        self.wifi.is_some()
    }
}

pub fn init(modem: Modem) -> Network {
    let wifi = start_wifi(modem);
    let sntp = if wifi.is_some() { start_sntp() } else { None };

    Network { wifi, _sntp: sntp }
}

fn start_wifi(modem: Modem) -> Option<BlockingWifi<EspWifi<'static>>> {
    let ssid = WIFI_SSID.unwrap_or("").trim();
    let pass = WIFI_PASS.unwrap_or("");

    if ssid.is_empty() {
        log::warn!("WIFI_SSID not set; skipping Wi-Fi login");
        return None;
    }
    if ssid.len() > 32 {
        log::error!("WIFI_SSID is too long; ESP Wi-Fi SSID limit is 32 bytes");
        return None;
    }
    if pass.len() > 64 {
        log::error!("WIFI_PASS is too long; ESP Wi-Fi password limit is 64 bytes");
        return None;
    }

    match connect_wifi(modem, ssid, pass) {
        Ok(wifi) => Some(wifi),
        Err(err) => {
            log::error!("Wi-Fi login failed: {err:?}");
            None
        }
    }
}

fn connect_wifi(
    modem: Modem,
    ssid: &str,
    pass: &str,
) -> Result<BlockingWifi<EspWifi<'static>>, EspError> {
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sys_loop.clone(), Some(nvs))?, sys_loop)?;

    let wifi_configuration = WifiConfiguration::Client(ClientConfiguration {
        ssid: ssid.try_into().unwrap(),
        bssid: None,
        auth_method: if pass.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        },
        password: pass.try_into().unwrap(),
        channel: None,
        ..Default::default()
    });

    wifi.set_configuration(&wifi_configuration)?;
    wifi.start()?;
    log::info!("Wi-Fi started; connecting to {ssid}");

    let mut last_err = None;
    for attempt in 1..=3 {
        log::info!("Wi-Fi connect attempt {attempt}/3");
        match wifi.connect().and_then(|_| wifi.wait_netif_up()) {
            Ok(()) => {
                let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
                log::info!("Wi-Fi connected: {:?}", ip_info);
                return Ok(wifi);
            }
            Err(err) => {
                log::warn!("Wi-Fi connect attempt {attempt}/3 failed: {err:?}");
                last_err = Some(err);
                let _ = wifi.disconnect();
                FreeRtos::delay_ms(1500);
            }
        }
    }

    Err(last_err.unwrap())
}

fn start_sntp() -> Option<EspSntp<'static>> {
    match EspSntp::new_default() {
        Ok(sntp) => {
            log::info!("SNTP started");
            for _ in 0..20 {
                if matches!(sntp.get_sync_status(), SyncStatus::Completed) {
                    log::info!("SNTP time synchronized");
                    return Some(sntp);
                }
                FreeRtos::delay_ms(500);
            }

            log::warn!("SNTP sync timed out; trying HTTPS health check anyway");
            Some(sntp)
        }
        Err(err) => {
            log::warn!("SNTP start failed: {err:?}; trying HTTPS health check anyway");
            None
        }
    }
}
