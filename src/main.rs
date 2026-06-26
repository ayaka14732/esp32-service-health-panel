// src/main.rs
// ESP32-S3 + ST7789 1.54" 240x240 IPS LCD 最小測試
//
// 接線：
//   LCD VCC  -> 3V3
//   LCD GND  -> GND
//   LCD SCL  -> GPIO12  (SPI SCLK)
//   LCD SDA  -> GPIO11  (SPI MOSI)
//   LCD RES  -> GPIO10  (Reset)
//   LCD DC   -> GPIO9   (Data/Command)
//   LCD CS   -> GPIO8   (Chip Select)
//   LCD BL   -> 3V3     (背光，常亮)

use esp_idf_hal::{
    delay::FreeRtos,
    gpio::{Gpio10, Gpio12, Gpio8, Gpio9, Output, PinDriver},
    modem::Modem,
    prelude::*,
    spi::{
        config::{Config as SpiConfig, Mode, Phase, Polarity},
        Dma, SpiBusDriver, SpiDriver,
    },
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::EspDefaultNvsPartition,
    wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi},
};
use esp_idf_sys as _; // 必須 link ESP-IDF runtime

// ───────────────────────────────────────────────
// ST7789 指令常數
// ───────────────────────────────────────────────
const ST7789_SWRESET: u8 = 0x01; // Software Reset
const ST7789_SLPOUT: u8 = 0x11; // Sleep Out
const ST7789_NORON: u8 = 0x13; // Normal Display Mode ON
const ST7789_INVON: u8 = 0x21; // Display Inversion ON（IPS 面板需要）
const ST7789_DISPON: u8 = 0x29; // Display ON
const ST7789_CASET: u8 = 0x2A; // Column Address Set
const ST7789_RASET: u8 = 0x2B; // Row Address Set
const ST7789_RAMWR: u8 = 0x2C; // Memory Write
const ST7789_COLMOD: u8 = 0x3A; // Interface Pixel Format
const ST7789_MADCTL: u8 = 0x36; // Memory Access Control

// 顏色（RGB565 格式，big-endian）
const RED: u16 = 0xF800;
const GREEN: u16 = 0x07E0;

// 屏幕尺寸
const LCD_W: u16 = 240;
const LCD_H: u16 = 240;

const WIFI_SSID: Option<&str> = option_env!("WIFI_SSID");
const WIFI_PASS: Option<&str> = option_env!("WIFI_PASS");

// ───────────────────────────────────────────────
// ST7789 驅動結構體
// ───────────────────────────────────────────────
struct St7789<'d> {
    spi: SpiBusDriver<'d, SpiDriver<'d>>,
    dc: PinDriver<'d, Gpio9, Output>,
    rst: PinDriver<'d, Gpio10, Output>,
    cs: PinDriver<'d, Gpio8, Output>,
}

impl<'d> St7789<'d> {
    /// 發送指令（DC 拉低）
    fn send_cmd(&mut self, cmd: u8) {
        self.dc.set_low().unwrap();
        self.cs.set_low().unwrap();
        self.spi.write(&[cmd]).unwrap();
        self.cs.set_high().unwrap();
    }

    /// 發送資料（DC 拉高）
    fn send_data(&mut self, data: &[u8]) {
        self.dc.set_high().unwrap();
        self.cs.set_low().unwrap();
        // SPI write 有 buffer 限制，分塊發送
        for chunk in data.chunks(64) {
            self.spi.write(chunk).unwrap();
        }
        self.cs.set_high().unwrap();
    }

    /// 發送單 byte 資料
    fn send_data_byte(&mut self, byte: u8) {
        self.send_data(&[byte]);
    }

    /// 硬件 Reset + 初始化序列
    fn init(&mut self) {
        // 硬件 Reset
        self.rst.set_high().unwrap();
        FreeRtos::delay_ms(10);
        self.rst.set_low().unwrap();
        FreeRtos::delay_ms(10);
        self.rst.set_high().unwrap();
        FreeRtos::delay_ms(120); // ST7789 datasheet: reset 後等 120ms

        // Software Reset
        self.send_cmd(ST7789_SWRESET);
        FreeRtos::delay_ms(150);

        // Sleep Out
        self.send_cmd(ST7789_SLPOUT);
        FreeRtos::delay_ms(500);

        // 像素格式：16-bit RGB565
        self.send_cmd(ST7789_COLMOD);
        self.send_data_byte(0x55); // 0x55 = 16bpp
        FreeRtos::delay_ms(10);

        // Memory Access Control
        // 0x00 = 正常方向（RGB order, top-to-bottom, left-to-right）
        self.send_cmd(ST7789_MADCTL);
        self.send_data_byte(0x00);

        // IPS 面板需要 Inversion ON
        self.send_cmd(ST7789_INVON);
        FreeRtos::delay_ms(10);

        // Normal Display Mode
        self.send_cmd(ST7789_NORON);
        FreeRtos::delay_ms(10);

        // Display ON
        self.send_cmd(ST7789_DISPON);
        FreeRtos::delay_ms(10);

        log::info!("ST7789 init done");
    }

    /// 設定寫入視窗（Column + Row Address）
    fn set_window(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) {
        // Column Address Set
        self.send_cmd(ST7789_CASET);
        self.send_data(&[
            (x0 >> 8) as u8,
            (x0 & 0xFF) as u8,
            (x1 >> 8) as u8,
            (x1 & 0xFF) as u8,
        ]);
        // Row Address Set
        self.send_cmd(ST7789_RASET);
        self.send_data(&[
            (y0 >> 8) as u8,
            (y0 & 0xFF) as u8,
            (y1 >> 8) as u8,
            (y1 & 0xFF) as u8,
        ]);
        // Memory Write（後面跟像素資料）
        self.send_cmd(ST7789_RAMWR);
    }

    /// 填充整個屏幕為單色
    fn fill_screen(&mut self, color: u16) {
        self.set_window(0, 0, LCD_W - 1, LCD_H - 1);

        let hi = (color >> 8) as u8;
        let lo = (color & 0xFF) as u8;

        // 每次發送 64 像素（128 bytes），減少 SPI 開銷
        // 240*240 = 57600 像素 = 115200 bytes
        let row: [u8; 128] = {
            let mut buf = [0u8; 128];
            for i in (0..128).step_by(2) {
                buf[i] = hi;
                buf[i + 1] = lo;
            }
            buf
        };

        self.dc.set_high().unwrap();
        self.cs.set_low().unwrap();
        // 57600 像素 / 64 像素 = 900 次
        for _ in 0..900 {
            self.spi.write(&row).unwrap();
        }
        self.cs.set_high().unwrap();
    }
}

// ───────────────────────────────────────────────
// Wi-Fi
// ───────────────────────────────────────────────
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
) -> Result<BlockingWifi<EspWifi<'static>>, esp_idf_sys::EspError> {
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sys_loop.clone(), Some(nvs))?, sys_loop)?;

    let wifi_configuration = Configuration::Client(ClientConfiguration {
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

    wifi.connect()?;
    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
    log::info!("Wi-Fi connected: {:?}", ip_info);

    Ok(wifi)
}

// ───────────────────────────────────────────────
// main
// ───────────────────────────────────────────────
fn main() {
    // 必須在最開頭呼叫，初始化 ESP-IDF runtime
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("ESP32-S3 LCD test starting...");

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;
    let modem = peripherals.modem;

    // SPI 驅動：SPI2（HSPI），40 MHz
    // ST7789 最高支持 ~62.5 MHz，保守用 40 MHz
    let spi_driver = SpiDriver::new(
        peripherals.spi2,
        pins.gpio12,                                                  // SCLK
        pins.gpio11,                                                  // MOSI
        None::<Gpio12>,                                               // MISO（不需要，LCD 單向）
        &esp_idf_hal::spi::SpiDriverConfig::new().dma(Dma::Disabled), // 最小測試不用 DMA
    )
    .unwrap();

    let spi_config = SpiConfig::new().baudrate(40.MHz().into()).data_mode(Mode {
        polarity: Polarity::IdleLow,            // CPOL=0
        phase: Phase::CaptureOnFirstTransition, // CPHA=0
    });
    // ST7789 用 SPI Mode 0（CPOL=0, CPHA=0）或 Mode 3 均可，
    // 這裡用 Mode 0

    let spi_bus = SpiBusDriver::new(spi_driver, &spi_config).unwrap();

    // GPIO 配置
    let dc = PinDriver::output(pins.gpio9).unwrap(); // Data/Command
    let rst = PinDriver::output(pins.gpio10).unwrap(); // Reset
    let cs = PinDriver::output(pins.gpio8).unwrap(); // Chip Select

    let mut lcd = St7789 {
        spi: spi_bus,
        dc,
        rst,
        cs,
    };

    // ── 初始化 LCD ──
    lcd.init();

    // ── Wi-Fi 登錄 ──
    // 保留 wifi 句柄，避免連線成功後 Wi-Fi driver 被 drop 而斷線。
    let wifi = start_wifi(modem);
    if wifi.is_some() {
        log::info!("Display status: Wi-Fi OK");
        lcd.fill_screen(GREEN);
    } else {
        log::info!("Display status: Wi-Fi failed");
        lcd.fill_screen(RED);
    }

    loop {
        FreeRtos::delay_ms(1000);
    }
}
