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

## 測試序列說明

開機後會依序顯示：
1. **全紅** → 確認 LCD 有輸出
2. **全綠** → 確認綠色通道
3. **全藍** → 確認藍色通道
4. **8色條** → 確認顏色正確
5. **棋盤格** → 確認像素對齊
6. **邊框+對角線** → 確認 240x240 覆蓋
7. **Service status 模擬** → 最終目標預覽

之後無限循環顯示 4/5/7。

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

## 下一步（Wi-Fi + HTTPS）

最小 LCD 測試通過後，下一步加入：
1. `esp-idf-svc::wifi` — Wi-Fi 連接
2. `esp-idf-svc::http::client` — HTTPS GET
3. 定時器每 60s 輪詢 8 個 endpoint
4. 結果寫入 LCD（不再需要 test patterns）
