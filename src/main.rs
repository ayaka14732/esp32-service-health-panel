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

mod health;
mod persian_status;

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
    sntp::{EspSntp, SyncStatus},
    wifi::{
        AuthMethod, BlockingWifi, ClientConfiguration, Configuration as WifiConfiguration, EspWifi,
    },
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
const BLACK: u16 = 0x0000;
const WHITE: u16 = 0xFFFF;
const RED: u16 = 0xF800;
const GREEN: u16 = 0x07E0;
const BLUE: u16 = 0x001F;

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

    fn draw_boot_screen(&mut self) {
        self.fill_screen(BLUE);
        self.draw_alpha_bitmap(
            persian_status::TITLE_X,
            persian_status::TITLE_Y,
            &persian_status::TITLE,
            BLUE,
            WHITE,
        );
    }

    fn draw_success_text(&mut self) {
        self.draw_alpha_bitmap(
            persian_status::TITLE_X,
            persian_status::TITLE_Y,
            &persian_status::TITLE,
            BLACK,
            WHITE,
        );

        for item in persian_status::STATUS_ITEMS.iter() {
            self.draw_filled_circle(item.circle_x, item.circle_y, 12, GREEN);
            self.draw_alpha_bitmap(item.text_x, item.text_y, item.label, BLACK, WHITE);
        }
    }

    fn draw_alpha_bitmap(
        &mut self,
        x: u16,
        y: u16,
        bitmap: &persian_status::AlphaBitmap,
        fg: u16,
        bg: u16,
    ) {
        if x >= LCD_W || y >= LCD_H {
            return;
        }

        let width = bitmap.width.min(LCD_W - x);
        let height = bitmap.height.min(LCD_H - y);
        let mut row_buf = [0u8; 480];

        for row in 0..height {
            self.set_window(x, y + row, x + width - 1, y + row);

            for col in 0..width {
                let alpha = bitmap_alpha(bitmap, col, row);
                let color = blend_rgb565(bg, fg, alpha);
                let offset = col as usize * 2;
                row_buf[offset] = (color >> 8) as u8;
                row_buf[offset + 1] = (color & 0xFF) as u8;
            }

            self.send_data(&row_buf[..width as usize * 2]);
        }
    }

    fn draw_filled_circle(&mut self, cx: u16, cy: u16, radius: u16, color: u16) {
        let cx = cx as i32;
        let cy = cy as i32;
        let radius = radius as i32;
        let radius_sq = radius * radius;

        for dy in -radius..=radius {
            let mut dx = radius;
            while dx * dx + dy * dy > radius_sq {
                dx -= 1;
            }

            let x0 = cx - dx;
            let x1 = cx + dx;
            let y = cy + dy;
            self.draw_solid_hline(x0, y, x1 - x0 + 1, color);
        }
    }

    fn draw_solid_hline(&mut self, x: i32, y: i32, len: i32, color: u16) {
        if len <= 0 || y < 0 || y >= LCD_H as i32 {
            return;
        }

        let x0 = x.max(0);
        let x1 = (x + len - 1).min(LCD_W as i32 - 1);
        if x0 > x1 {
            return;
        }

        let width = (x1 - x0 + 1) as u16;
        self.set_window(x0 as u16, y as u16, x1 as u16, y as u16);

        let hi = (color >> 8) as u8;
        let lo = (color & 0xFF) as u8;
        let mut row_buf = [0u8; 480];
        for offset in (0..width as usize * 2).step_by(2) {
            row_buf[offset] = hi;
            row_buf[offset + 1] = lo;
        }

        self.send_data(&row_buf[..width as usize * 2]);
    }
}

fn bitmap_alpha(bitmap: &persian_status::AlphaBitmap, x: u16, y: u16) -> u8 {
    let index = y as usize * bitmap.width as usize + x as usize;
    let byte = bitmap.data[index / 2];
    if index % 2 == 0 {
        byte >> 4
    } else {
        byte & 0x0F
    }
}

fn blend_rgb565(bg: u16, fg: u16, alpha: u8) -> u16 {
    if alpha == 0 {
        return bg;
    }
    if alpha >= 15 {
        return fg;
    }

    let alpha = alpha as u16;
    let inv = 15 - alpha;

    let bg_r = (bg >> 11) & 0x1F;
    let bg_g = (bg >> 5) & 0x3F;
    let bg_b = bg & 0x1F;
    let fg_r = (fg >> 11) & 0x1F;
    let fg_g = (fg >> 5) & 0x3F;
    let fg_b = fg & 0x1F;

    let r = (fg_r * alpha + bg_r * inv) / 15;
    let g = (fg_g * alpha + bg_g * inv) / 15;
    let b = (fg_b * alpha + bg_b * inv) / 15;

    (r << 11) | (g << 5) | b
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

    // ── 開機畫面：立即顯示，避免 Wi-Fi 連線期間屏幕雜訊 ──
    lcd.draw_boot_screen();

    // ── Wi-Fi 登錄 ──
    // 保留 wifi 句柄，避免連線成功後 Wi-Fi driver 被 drop 而斷線。
    let wifi = start_wifi(modem);
    let _sntp = if wifi.is_some() { start_sntp() } else { None };
    let railway_ok = wifi.is_some() && health::railway::ok();
    let ipinfo_ok = wifi.is_some() && health::ipinfo::ok();
    let healthy = railway_ok && ipinfo_ok;

    if healthy {
        log::info!("Display status: all health checks passed");
        lcd.fill_screen(WHITE);
        lcd.draw_success_text();
    } else {
        log::info!("Display status: health check failed");
        lcd.fill_screen(RED);
    }

    loop {
        FreeRtos::delay_ms(1000);
    }
}
