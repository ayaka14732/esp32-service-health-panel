// src/main.rs
// ESP32-S3 + ST7789 1.54" 240x240 IPS LCD service health panel
//
// Wiring:
//   LCD VCC  -> 3V3
//   LCD GND  -> GND
//   LCD SCL  -> GPIO12  (SPI SCLK)
//   LCD SDA  -> GPIO11  (SPI MOSI)
//   LCD RES  -> GPIO10  (Reset)
//   LCD DC   -> GPIO9   (Data/Command)
//   LCD CS   -> GPIO8   (Chip Select)
//   LCD BL   -> 3V3     (backlight, always on)

mod health;
mod network;
mod persian_status;

use esp_idf_hal::{
    delay::FreeRtos,
    gpio::{Gpio10, Gpio12, Gpio8, Gpio9, Output, PinDriver},
    prelude::*,
    spi::{
        config::{Config as SpiConfig, Mode, Phase, Polarity},
        Dma, SpiBusDriver, SpiDriver,
    },
};
use esp_idf_sys as _; // Required to link the ESP-IDF runtime

// ───────────────────────────────────────────────
// ST7789 command constants
// ───────────────────────────────────────────────
const ST7789_SWRESET: u8 = 0x01; // Software Reset
const ST7789_SLPOUT: u8 = 0x11; // Sleep Out
const ST7789_NORON: u8 = 0x13; // Normal Display Mode ON
const ST7789_INVON: u8 = 0x21; // Display Inversion ON (needed by IPS panels)
const ST7789_DISPON: u8 = 0x29; // Display ON
const ST7789_CASET: u8 = 0x2A; // Column Address Set
const ST7789_RASET: u8 = 0x2B; // Row Address Set
const ST7789_RAMWR: u8 = 0x2C; // Memory Write
const ST7789_COLMOD: u8 = 0x3A; // Interface Pixel Format
const ST7789_MADCTL: u8 = 0x36; // Memory Access Control

// Colors in RGB565 format, big-endian
const BLACK: u16 = 0x0000;
const WHITE: u16 = 0xFFFF;
const RED: u16 = 0xF800;
const GREEN: u16 = 0x07E0;
const BLUE: u16 = 0x001F;

// Screen dimensions
const LCD_W: u16 = 240;
const LCD_H: u16 = 240;

const HEALTH_CHECK_INTERVAL_MS: u32 = 10 * 60 * 1000;

// ───────────────────────────────────────────────
// ST7789 driver state
// ───────────────────────────────────────────────
struct St7789<'d> {
    spi: SpiBusDriver<'d, SpiDriver<'d>>,
    dc: PinDriver<'d, Gpio9, Output>,
    rst: PinDriver<'d, Gpio10, Output>,
    cs: PinDriver<'d, Gpio8, Output>,
}

impl<'d> St7789<'d> {
    /// Send a command with DC low.
    fn send_cmd(&mut self, cmd: u8) {
        self.dc.set_low().unwrap();
        self.cs.set_low().unwrap();
        self.spi.write(&[cmd]).unwrap();
        self.cs.set_high().unwrap();
    }

    /// Send data with DC high.
    fn send_data(&mut self, data: &[u8]) {
        self.dc.set_high().unwrap();
        self.cs.set_low().unwrap();
        // SPI writes have a buffer limit, so send data in chunks.
        for chunk in data.chunks(64) {
            self.spi.write(chunk).unwrap();
        }
        self.cs.set_high().unwrap();
    }

    /// Send a single data byte.
    fn send_data_byte(&mut self, byte: u8) {
        self.send_data(&[byte]);
    }

    /// Hardware reset and initialization sequence.
    fn init(&mut self) {
        // Hardware reset
        self.rst.set_high().unwrap();
        FreeRtos::delay_ms(10);
        self.rst.set_low().unwrap();
        FreeRtos::delay_ms(10);
        self.rst.set_high().unwrap();
        FreeRtos::delay_ms(120); // ST7789 datasheet: wait 120 ms after reset

        // Software Reset
        self.send_cmd(ST7789_SWRESET);
        FreeRtos::delay_ms(150);

        // Sleep Out
        self.send_cmd(ST7789_SLPOUT);
        FreeRtos::delay_ms(500);

        // Pixel format: 16-bit RGB565
        self.send_cmd(ST7789_COLMOD);
        self.send_data_byte(0x55); // 0x55 = 16bpp
        FreeRtos::delay_ms(10);

        // Memory Access Control
        // 0x00 = normal orientation (RGB order, top-to-bottom, left-to-right)
        self.send_cmd(ST7789_MADCTL);
        self.send_data_byte(0x00);

        // IPS panels need inversion enabled.
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

    /// Set the write window (column and row address).
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
        // Memory Write, followed by pixel data.
        self.send_cmd(ST7789_RAMWR);
    }

    /// Fill the entire screen with one color.
    fn fill_screen(&mut self, color: u16) {
        self.set_window(0, 0, LCD_W - 1, LCD_H - 1);

        let hi = (color >> 8) as u8;
        let lo = (color & 0xFF) as u8;

        // Send 64 pixels (128 bytes) at a time to reduce SPI overhead.
        // 240*240 = 57,600 pixels = 115,200 bytes.
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
        // 57,600 pixels / 64 pixels = 900 writes.
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
            WHITE,
            BLUE,
        );
    }

    fn draw_status_screen(&mut self, results: [bool; 4]) {
        self.fill_screen(WHITE);
        self.draw_alpha_bitmap(
            persian_status::TITLE_X,
            persian_status::TITLE_Y,
            &persian_status::TITLE,
            BLACK,
            WHITE,
        );

        for (item, ok) in persian_status::STATUS_ITEMS.iter().zip(results.iter()) {
            let color = if *ok { GREEN } else { RED };
            self.draw_filled_circle(item.circle_x, item.circle_y, 12, color);
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
// main
// ───────────────────────────────────────────────
fn main() {
    // Must be called first to initialize the ESP-IDF runtime.
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("ESP32-S3 LCD test starting...");

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;
    let modem = peripherals.modem;

    // SPI driver: SPI2 (HSPI), 40 MHz.
    // ST7789 supports up to about 62.5 MHz; use 40 MHz conservatively.
    let spi_driver = SpiDriver::new(
        peripherals.spi2,
        pins.gpio12,                                                  // SCLK
        pins.gpio11,                                                  // MOSI
        None::<Gpio12>, // MISO is not needed; the LCD is write-only.
        &esp_idf_hal::spi::SpiDriverConfig::new().dma(Dma::Disabled), // DMA is not needed for this small display.
    )
    .unwrap();

    let spi_config = SpiConfig::new().baudrate(40.MHz().into()).data_mode(Mode {
        polarity: Polarity::IdleLow,            // CPOL=0
        phase: Phase::CaptureOnFirstTransition, // CPHA=0
    });
    // ST7789 works with SPI Mode 0 (CPOL=0, CPHA=0) or Mode 3.
    // Use Mode 0 here.

    let spi_bus = SpiBusDriver::new(spi_driver, &spi_config).unwrap();

    // GPIO configuration
    let dc = PinDriver::output(pins.gpio9).unwrap(); // Data/Command
    let rst = PinDriver::output(pins.gpio10).unwrap(); // Reset
    let cs = PinDriver::output(pins.gpio8).unwrap(); // Chip Select

    let mut lcd = St7789 {
        spi: spi_bus,
        dc,
        rst,
        cs,
    };

    // ── Initialize LCD ──
    lcd.init();

    // ── Boot screen: show immediately to avoid display noise during Wi-Fi connection ──
    lcd.draw_boot_screen();

    // ── Network login ──
    // Keep the network handle alive so Wi-Fi and SNTP are not dropped.
    let network = network::init(modem);

    loop {
        let online = network.is_connected();
        let railway_ok = online && health::railway::ok();
        let ipinfo_ok = online && health::ipinfo::ok();
        let graphviz_ok = online && health::graphviz::ok();
        let polaris_ok = online && health::polaris::ok();

        log::info!(
            "Display status: railway={railway_ok}, ipinfo={ipinfo_ok}, graphviz={graphviz_ok}, polaris={polaris_ok}"
        );
        lcd.draw_status_screen([railway_ok, ipinfo_ok, graphviz_ok, polaris_ok]);

        FreeRtos::delay_ms(HEALTH_CHECK_INTERVAL_MS);
    }
}
