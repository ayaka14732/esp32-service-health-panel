# ESP32-S3 + ST7789 LCD Test

## 硬件接線

| LCD Pin | ESP32-S3 GPIO |
|---------|--------------|
| VCC     | 3V3          |
| GND     | GND          |
| SCL/SCK | GPIO12       |
| SDA/MOSI| GPIO11       |
| RES/RST | GPIO10       |
| DC/A0   | GPIO9        |
| CS      | GPIO8        |
| BL      | 3V3 (常亮)   |

---

## Arch Linux 環境安裝

### 1. 安裝 espup（ESP32 Rust toolchain 管理器）

```bash
sudo pacman -S --needed libxml2-legacy  # ESP-IDF clang 需要舊版 libxml2 ABI
cargo install espup
espup install   # 安裝 xtensa-esp32s3-espidf toolchain
source ~/export-esp.sh  # 每次開新 terminal 都需要
```

本專案包含 `rust-toolchain.toml`，會自動使用 `esp` Rust toolchain。

### 2. 安裝 espflash

```bash
cargo install espflash
cargo install ldproxy
```

### 3. serial port 權限（避免每次 sudo）

```bash
sudo usermod -aG uucp $USER  # Arch 通常使用 uucp；其他 distro 可能是 dialout
# 重新登入生效
# 確認：
ls -la /dev/ttyACM0
```

### 4. 確認 ESP-IDF（espup 會自動安裝）

```bash
# espup 裝完後路徑通常在 ~/.espressif/
# 確認 export-esp.sh 存在：
ls ~/export-esp.sh
```

---

## Build & Flash

### Wi-Fi 登錄設定

Wi-Fi SSID / password 寫在 `.env`，建議先從範本複製：

```bash
cp .env.example .env
```

`.env` 內容：

```dotenv
WIFI_SSID=你的 Wi-Fi 名稱
WIFI_PASS=你的 Wi-Fi 密碼
```

若是開放網路，可把 `WIFI_PASS` 留空。若沒有設定 `WIFI_SSID`，韌體會判定
Wi-Fi 登錄失敗並顯示紅色。

重要：ESP32-S3 只支援 **2.4 GHz Wi-Fi**，不能連 **5 GHz Wi-Fi**。如果路由器有
`xxx_5G` / `xxx_2G` 兩個 SSID，請在 `.env` 裡填 2.4 GHz 那個，例如 `xxx_2G`。
連到 5 GHz SSID 會 timeout，serial monitor 可能會看到 `ESP_ERR_TIMEOUT`。

```bash
# 進入專案目錄
cd esp32-lcd-test

# 每次開新 terminal 先 source
source ~/export-esp.sh

# Build（第一次很慢，要編 ESP-IDF）
cargo build --release

# Flash app + 開 serial monitor
# 若 bootloader 已經在板子上，平常用這個即可
cargo run --release

# 或分開操作，只更新 app：
cargo build --release
espflash flash --flash-mode dout --flash-freq 20mhz --flash-size 16mb \
  target/xtensa-esp32s3-espidf/release/esp32-lcd-test --monitor
```

連線成功後 serial monitor 會顯示 `Wi-Fi connected` 與 DHCP IP 資訊，接著會檢查兩個 endpoint：

```text
https://uk-railway-journey-recorder-api.shn.hk/api/health
https://ipinfo.shn.hk/
```

第一個 endpoint 必須回傳 HTTP 200 且 body 是 `{"status":"ok"}`。
第二個 endpoint 必須回傳 HTTP 200 且 response 第一個字符是數字。
只有兩個 health check 都成功時，LCD 才會顯示純綠色。若 Wi-Fi 登錄失敗、沒有設定 SSID、
HTTPS 請求失敗、或任一 health check 不符合條件，LCD 都會顯示純紅色。
因為 `.env` 內容會編譯進韌體，改 SSID / password 後請重新 `cargo run --release`。

本板子的 embedded flash 使用 DIO/40 MHz 會在 ROM 階段出現
`Invalid image block, can't boot.`。`sdkconfig.defaults` 已固定為 DOUT/20 MHz。

如果剛 erase flash、換板子、或再次看到 bootloader 錯誤，請先完整寫入
bootloader、partition table 和 app：

```bash
source ~/export-esp.sh
cargo build --release

PROJECT_DIR=$PWD
IDF_BUILD_DIR=$(
  find target/xtensa-esp32s3-espidf/release/build -path '*/out/build/flash_args' \
    -printf '%T@ %h\n' | sort -n | tail -1 | cut -d' ' -f2-
)

sudo espflash flash \
  --chip esp32s3 --port /dev/ttyACM0 \
  --flash-mode dout --flash-freq 20mhz --flash-size 16mb \
  --bootloader "$IDF_BUILD_DIR/bootloader/bootloader.bin" \
  --partition-table "$PROJECT_DIR/.embuild/espressif/esp-idf/v5.2.3/components/partition_table/partitions_singleapp.csv" \
  --partition-table-offset 0x8000 \
  --monitor target/xtensa-esp32s3-espidf/release/esp32-lcd-test
```

---

## 屏幕狀態

開機後只顯示 API health check 結果：
1. **白底 + 置中波斯語標題 + 右側綠色圓點狀態列** → Wi-Fi 登錄成功，且兩個 API health check 都通過
2. **純紅色** → Wi-Fi 或任一 API health check 失敗

成功畫面上的波斯語狀態文字為預渲染 bitmap：
1. 置中標題：`وضعیت سلامت`
2. `وای‌فای` + 右側綠色圓點
3. `راه‌آهن` + 右側綠色圓點
4. `آی‌پی` + 右側綠色圓點

波斯語 bitmap 由 `tools/generate_persian_status.py` 生成。若要改文字、字型或位置，修改
該 script 後重新生成：

```bash
python3 tools/generate_persian_status.py
cargo fmt
```

script 需要 Python Pillow，且 Pillow 需支援 RAQM shaping；字型預設使用
`NotoSansArabic-Bold.ttf`。生成後會更新 `src/persian_status.rs`，並輸出預覽圖到
`target/persian_status_preview.png`。

---

## 常見問題

### 畫面全白 / 全黑不變
- 確認 BL（背光）已接 3V3
- 確認 RES 接 GPIO10（不是直接接 3V3）
- 用示波器 / 邏輯分析儀確認 GPIO12 有 SPI 時鐘

### 顏色反轉（紅藍互換）
- 修改 `MADCTL` 值：`0x00` 改 `0x08`（RGB/BGR bit）

### 畫面上移 / 偏移
- 部分 1.54" ST7789 模組有 row/column offset
- 在 `set_window` 加 offset：
  ```rust
  // 若畫面偏移 (ox, oy)，例如 ox=0, oy=80
  const OFFSET_X: u16 = 0;
  const OFFSET_Y: u16 = 0;  // 先試 0，不對再改 80
  ```

### Build 時找不到 xtensa target
```bash
# 確認 toolchain 已安裝
rustup toolchain list | grep xtensa
# 確認 source 了 export-esp.sh
echo $PATH | grep xtensa
```

### espflash 找不到 port
```bash
espflash list-ports
espflash flash --port /dev/ttyACM0 target/.../esp32-lcd-test --monitor
```

---

## 下一步（HTTPS）

Wi-Fi 登錄已加入；下一步可以接：
1. `esp-idf-svc::http::client` — HTTPS GET
2. 定時器每 60s 輪詢 8 個 endpoint
3. 結果寫入 LCD（不再需要 test patterns）
