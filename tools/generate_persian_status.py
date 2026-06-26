#!/usr/bin/env python3
"""Generate pre-rendered Persian status labels for the ST7789 display."""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = PROJECT_ROOT / "src" / "persian_status.rs"
DEFAULT_PREVIEW = PROJECT_ROOT / "target" / "persian_status_preview.png"
FONT_CANDIDATES = [
    Path("/usr/share/fonts/noto/NotoSansArabic-Bold.ttf"),
    Path("/usr/share/fonts/truetype/noto/NotoSansArabic-Bold.ttf"),
]

TITLE = ("TITLE", "وضعیت سلامت", 29)
ITEMS = [
    ("RAILWAY", "راه\u200cآهن", 34),
    ("IPINFO", "آی\u200cپی", 34),
    ("GRAPHVIZ", "گراف\u200cویز", 34),
]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--font", type=Path, default=None)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--preview", type=Path, default=DEFAULT_PREVIEW)
    args = parser.parse_args()

    font_path = args.font or find_font()
    bitmaps = render_bitmaps(font_path)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(render_rust(bitmaps), encoding="utf-8")

    args.preview.parent.mkdir(parents=True, exist_ok=True)
    render_preview(font_path, args.preview)

    print(f"Generated {args.output}")
    print(f"Preview written to {args.preview}")


def find_font() -> Path:
    for path in FONT_CANDIDATES:
        if path.exists():
            return path

    raise SystemExit(
        "NotoSansArabic-Bold.ttf was not found. Install Noto Arabic fonts or pass --font."
    )


def render_bitmaps(font_path: Path) -> list[dict[str, object]]:
    results = []
    for name, text, size in [TITLE] + ITEMS:
        font = ImageFont.truetype(
            str(font_path),
            size,
            layout_engine=ImageFont.Layout.RAQM,
        )
        image = render_alpha_image(text, font)
        results.append(
            {
                "name": name,
                "text": text,
                "width": image.width,
                "height": image.height,
                "data": pack_alpha4(image),
            }
        )

    return results


def render_alpha_image(text: str, font: ImageFont.FreeTypeFont) -> Image.Image:
    scratch = Image.new("L", (280, 90), 0)
    draw = ImageDraw.Draw(scratch)
    bbox = draw.textbbox((0, 0), text, font=font, direction="rtl", language="fa")
    width = bbox[2] - bbox[0]
    height = bbox[3] - bbox[1]

    image = Image.new("L", (width + 4, height + 4), 0)
    draw = ImageDraw.Draw(image)
    draw.text(
        (2 - bbox[0], 2 - bbox[1]),
        text,
        fill=255,
        font=font,
        direction="rtl",
        language="fa",
    )

    return image


def pack_alpha4(image: Image.Image) -> list[int]:
    if hasattr(image, "get_flattened_data"):
        values = list(image.get_flattened_data())
    else:
        values = list(image.getdata())
    packed = []

    for index in range(0, len(values), 2):
        high = min(15, (values[index] + 8) // 17)
        low = min(15, (values[index + 1] + 8) // 17) if index + 1 < len(values) else 0
        packed.append((high << 4) | low)

    return packed


def render_rust(bitmaps: list[dict[str, object]]) -> str:
    dimensions = {item["name"]: (item["width"], item["height"]) for item in bitmaps}
    title_width = int(dimensions["TITLE"][0])
    title_x = (240 - title_width) // 2
    text_right = 160
    circle_x = 186

    lines = [
        "// Generated Persian status label masks. Source text:",
        "// Title    -> وضعیت سلامت",
        "// Railway  -> راه‌آهن",
        "// IP info  -> آی‌پی",
        "// Graphviz -> گراف‌ویز",
        "",
        "pub struct AlphaBitmap {",
        "    pub width: u16,",
        "    pub height: u16,",
        "    pub data: &'static [u8],",
        "}",
        "",
        "pub struct StatusItem {",
        "    pub circle_x: u16,",
        "    pub circle_y: u16,",
        "    pub text_x: u16,",
        "    pub text_y: u16,",
        "    pub label: &'static AlphaBitmap,",
        "}",
        "",
    ]

    for item in bitmaps:
        name = str(item["name"])
        data = item["data"]
        lines.append(f"const {name}_DATA: [u8; {len(data)}] = [")
        for index in range(0, len(data), 16):
            chunk = ", ".join(f"0x{byte:02X}" for byte in data[index : index + 16])
            lines.append(f"    {chunk},")
        lines.extend(
            [
                "];",
                "",
                f"pub const {name}: AlphaBitmap = AlphaBitmap {{",
                f"    width: {item['width']},",
                f"    height: {item['height']},",
                f"    data: &{name}_DATA,",
                "};",
                "",
            ]
        )

    lines.extend(
        [
            f"pub const TITLE_X: u16 = {title_x};",
            "pub const TITLE_Y: u16 = 16;",
            "",
            "pub const STATUS_ITEMS: [StatusItem; 3] = [",
        ]
    )

    for y, name in [(82, "RAILWAY"), (136, "IPINFO"), (190, "GRAPHVIZ")]:
        width, height = dimensions[name]
        lines.extend(
            [
                "    StatusItem {",
                f"        circle_x: {circle_x},",
                f"        circle_y: {y},",
                f"        text_x: {text_right - int(width)},",
                f"        text_y: {y - int(height) // 2},",
                f"        label: &{name},",
                "    },",
            ]
        )

    lines.extend(["];", ""])
    return "\n".join(lines)


def render_preview(font_path: Path, output: Path) -> None:
    title_font = ImageFont.truetype(
        str(font_path),
        TITLE[2],
        layout_engine=ImageFont.Layout.RAQM,
    )
    item_fonts = [
        ImageFont.truetype(str(font_path), size, layout_engine=ImageFont.Layout.RAQM)
        for _, _, size in ITEMS
    ]

    preview = Image.new("RGB", (240, 240), (255, 255, 255))
    draw = ImageDraw.Draw(preview)

    title_bbox = draw.textbbox(
        (0, 0),
        TITLE[1],
        font=title_font,
        direction="rtl",
        language="fa",
    )
    title_width = title_bbox[2] - title_bbox[0]
    draw.text(
        ((240 - title_width) // 2 - title_bbox[0], 16 - title_bbox[1]),
        TITLE[1],
        fill=(0, 0, 0),
        font=title_font,
        direction="rtl",
        language="fa",
    )

    for y, item, font in zip([82, 136, 190], ITEMS, item_fonts):
        text = item[1]
        draw.ellipse((174, y - 12, 198, y + 12), fill=(0, 180, 60))
        bbox = draw.textbbox((0, 0), text, font=font, direction="rtl", language="fa")
        width = bbox[2] - bbox[0]
        draw.text(
            (160 - width - bbox[0], y - (bbox[3] - bbox[1]) // 2 - bbox[1]),
            text,
            fill=(0, 0, 0),
            font=font,
            direction="rtl",
            language="fa",
        )

    preview.save(output)


if __name__ == "__main__":
    main()
