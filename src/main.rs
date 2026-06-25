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
    prelude::*,
    spi::{
        config::{Config as SpiConfig, Mode, Phase, Polarity},
        Dma, SpiBusDriver, SpiDriver,
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
const YELLOW: u16 = 0xFFE0;
const CYAN: u16 = 0x07FF;
const MAGENTA: u16 = 0xF81F;

// 屏幕尺寸
const LCD_W: u16 = 240;
const LCD_H: u16 = 240;

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

    /// 畫一個填充矩形
    fn fill_rect(&mut self, x: u16, y: u16, w: u16, h: u16, color: u16) {
        if x >= LCD_W || y >= LCD_H {
            return;
        }
        let x1 = (x + w - 1).min(LCD_W - 1);
        let y1 = (y + h - 1).min(LCD_H - 1);
        self.set_window(x, y, x1, y1);

        let hi = (color >> 8) as u8;
        let lo = (color & 0xFF) as u8;
        let pixels = (x1 - x + 1) as u32 * (y1 - y + 1) as u32;

        // 用 32-pixel 緩衝區發送
        let chunk: [u8; 64] = {
            let mut buf = [0u8; 64];
            for i in (0..64).step_by(2) {
                buf[i] = hi;
                buf[i + 1] = lo;
            }
            buf
        };

        self.dc.set_high().unwrap();
        self.cs.set_low().unwrap();
        let full_chunks = pixels / 32;
        let remainder = (pixels % 32) as usize;
        for _ in 0..full_chunks {
            self.spi.write(&chunk).unwrap();
        }
        if remainder > 0 {
            self.spi.write(&chunk[..remainder * 2]).unwrap();
        }
        self.cs.set_high().unwrap();
    }

    /// 畫單個像素
    fn draw_pixel(&mut self, x: u16, y: u16, color: u16) {
        if x >= LCD_W || y >= LCD_H {
            return;
        }
        self.set_window(x, y, x, y);
        self.send_data(&[(color >> 8) as u8, (color & 0xFF) as u8]);
    }

    /// 畫水平線（效率比逐點快）
    fn draw_hline(&mut self, x: u16, y: u16, len: u16, color: u16) {
        self.fill_rect(x, y, len, 1, color);
    }

    /// 畫垂直線
    fn draw_vline(&mut self, x: u16, y: u16, len: u16, color: u16) {
        self.fill_rect(x, y, 1, len, color);
    }

    /// 顯示顏色條測試圖案
    /// 分 8 格，每格不同顏色
    fn test_color_bars(&mut self) {
        let colors = [RED, GREEN, BLUE, YELLOW, CYAN, MAGENTA, WHITE, BLACK];
        let bar_w = LCD_W / 8; // 30 像素每條
        for (i, &color) in colors.iter().enumerate() {
            self.fill_rect(i as u16 * bar_w, 0, bar_w, LCD_H, color);
        }
        log::info!("Color bars drawn");
    }

    /// 顯示棋盤格測試圖案（測試像素對齊）
    fn test_checkerboard(&mut self) {
        let cell = 20u16; // 20x20 格子
        self.fill_screen(BLACK);
        let cols = LCD_W / cell;
        let rows = LCD_H / cell;
        for row in 0..rows {
            for col in 0..cols {
                let color = if (row + col) % 2 == 0 { WHITE } else { BLACK };
                self.fill_rect(col * cell, row * cell, cell, cell, color);
            }
        }
        log::info!("Checkerboard drawn");
    }

    /// 顯示邊框測試（確認 240x240 範圍）
    fn test_border(&mut self) {
        self.fill_screen(BLACK);
        // 外框：2px 白色
        self.draw_hline(0, 0, LCD_W, WHITE); // 上
        self.draw_hline(0, LCD_H - 1, LCD_W, WHITE); // 下
        self.draw_vline(0, 0, LCD_H, WHITE); // 左
        self.draw_vline(LCD_W - 1, 0, LCD_H, WHITE); // 右
                                                     // 對角線（像素點）
        for i in 0..LCD_W.min(LCD_H) {
            self.draw_pixel(i, i, RED);
            self.draw_pixel(LCD_W - 1 - i, i, GREEN);
        }
        log::info!("Border test drawn");
    }

    /// service status 模擬畫面（最終目標預覽）
    /// 8 個格子，顯示 OK（綠）或 DOWN（紅）
    fn test_service_status(&mut self) {
        self.fill_screen(BLACK);

        // 2x4 grid，每個 120x60
        let cell_w: u16 = 120;
        let cell_h: u16 = 60;
        let services = [
            ("SVC-1", true),
            ("SVC-2", true),
            ("SVC-3", false),
            ("SVC-4", true),
            ("SVC-5", true),
            ("SVC-6", false),
            ("SVC-7", true),
            ("SVC-8", true),
        ];

        for (i, (_name, ok)) in services.iter().enumerate() {
            let col = (i % 2) as u16;
            let row = (i / 2) as u16;
            let x = col * cell_w;
            let y = row * cell_h;
            let bg = if *ok { 0x0400u16 } else { 0x6000u16 }; // 深綠 / 深紅背景
            let fg = if *ok { GREEN } else { RED };

            // 背景
            self.fill_rect(x + 2, y + 2, cell_w - 4, cell_h - 4, bg);
            // 狀態指示條（左側 8px）
            self.fill_rect(x + 2, y + 2, 8, cell_h - 4, fg);
            // 分格線
            self.draw_hline(x, y, cell_w, 0x2104); // 暗灰
            self.draw_vline(x, y, cell_h, 0x2104);
        }
        log::info!("Service status mock drawn");
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

    // ── 測試序列 ──
    // 1. 全屏紅色（確認 LCD 有輸出）
    log::info!("Test 1: Full red");
    lcd.fill_screen(RED);
    FreeRtos::delay_ms(1500);

    // 2. 全屏綠色
    log::info!("Test 2: Full green");
    lcd.fill_screen(GREEN);
    FreeRtos::delay_ms(1500);

    // 3. 全屏藍色
    log::info!("Test 3: Full blue");
    lcd.fill_screen(BLUE);
    FreeRtos::delay_ms(1500);

    // 4. 顏色條（8色）
    log::info!("Test 4: Color bars");
    lcd.test_color_bars();
    FreeRtos::delay_ms(2000);

    // 5. 棋盤格（測試像素精度）
    log::info!("Test 5: Checkerboard");
    lcd.test_checkerboard();
    FreeRtos::delay_ms(2000);

    // 6. 邊框 + 對角線（確認 240x240 覆蓋）
    log::info!("Test 6: Border + diagonals");
    lcd.test_border();
    FreeRtos::delay_ms(2000);

    // 7. Service status 模擬（最終目標預覽）
    log::info!("Test 7: Service status mock");
    lcd.test_service_status();
    FreeRtos::delay_ms(3000);

    // ── 循環展示 ──
    log::info!("Entering display loop");
    let mut step: u32 = 0;
    loop {
        match step % 3 {
            0 => lcd.test_color_bars(),
            1 => lcd.test_checkerboard(),
            _ => lcd.test_service_status(),
        }
        step += 1;
        FreeRtos::delay_ms(3000);
    }
}
