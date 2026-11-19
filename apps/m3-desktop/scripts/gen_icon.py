#!/usr/bin/env python3
"""Generate the black matte squircle + white pixel-art 'M3' macOS AppIcon.icns."""

from __future__ import annotations

import shutil
import struct
import subprocess
import zlib
from pathlib import Path

BITMAP = [
    "##....##..#####.",
    "###..###..######",
    "########......##",
    "##.##.##......##",
    "##.##.##....###.",
    "##....##....###.",
    "##....##......##",
    "##....##......##",
    "##....##..######",
    "##....##..#####.",
]

BG_RGBA = (11, 11, 14, 255)      # #0B0B0E matte black
RIM_RGBA = (36, 36, 42, 255)     # #24242A subtle inner rim for dark mode Dock legibility
FG_RGBA = (255, 255, 255, 255)   # #FFFFFF pure white pixel art


def write_png(path: Path, width: int, height: int, rgba_rows: list[bytes]) -> None:
    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    raw = b"".join(b"\x00" + row for row in rgba_rows)
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, level=9))
        + chunk(b"IEND", b"")
    )
    path.write_bytes(png)


def squircle_dist(nx: float, ny: float, power: float = 4.8) -> float:
    return (abs(nx) ** power + abs(ny) ** power) ** (1.0 / power)


def render_icon(size: int) -> list[bytes]:
    cols = len(BITMAP[0])  # 16
    rows = len(BITMAP)     # 10

    # Standard macOS icon squircle fills ~824/1024 (~80.47%) of canvas
    half_extent = (size * 824.0 / 1024.0) / 2.0
    center = size / 2.0

    use_integer_cells = size >= 64
    if use_integer_cells:
        cell = max(1, int(round(36.0 * size / 1024.0)))
        if size == 128:
            cell = 4
        art_w = cols * cell
        art_h = rows * cell
        ox = (size - art_w) // 2
        oy = (size - art_h) // 2
    else:
        art_w_f = size * (576.0 / 1024.0)
        art_h_f = size * (360.0 / 1024.0)
        ox_f = (size - art_w_f) / 2.0
        oy_f = (size - art_h_f) / 2.0

    # Supersample 4x4 per pixel for clean squircle edges (and crisp pixel glyph)
    ss = 4
    inv_ss = 1.0 / ss
    samples = ss * ss
    rim_threshold = 1.0 - max(1.5 / half_extent, 0.008)

    out_rows: list[bytes] = []
    for y in range(size):
        row_buf = bytearray(size * 4)
        for x in range(size):
            r_sum = g_sum = b_sum = a_sum = 0.0
            for sy in range(ss):
                py = y + (sy + 0.5) * inv_ss
                ny = (py - center) / half_extent
                for sx in range(ss):
                    px = x + (sx + 0.5) * inv_ss
                    nx = (px - center) / half_extent
                    d = squircle_dist(nx, ny)
                    if d > 1.0:
                        continue

                    if use_integer_cells:
                        gx = int(px) - ox
                        gy = int(py) - oy
                        in_glyph = (
                            0 <= gx < art_w
                            and 0 <= gy < art_h
                            and BITMAP[gy // cell][gx // cell] == "#"
                        )
                    else:
                        fx = (px - ox_f) / art_w_f * cols
                        fy = (py - oy_f) / art_h_f * rows
                        cx_i = int(fx)
                        cy_i = int(fy)
                        in_glyph = (
                            0.0 <= fx < cols
                            and 0.0 <= fy < rows
                            and BITMAP[cy_i][cx_i] == "#"
                        )

                    if in_glyph:
                        cr, cg, cb, ca = FG_RGBA
                    elif d >= rim_threshold:
                        cr, cg, cb, ca = RIM_RGBA
                    else:
                        cr, cg, cb, ca = BG_RGBA

                    r_sum += cr
                    g_sum += cg
                    b_sum += cb
                    a_sum += ca

            idx = x * 4
            if a_sum > 0.0:
                # Straight RGBA: unpremultiply RGB by sample alpha coverage
                cov = a_sum / 255.0
                row_buf[idx] = min(255, int(round(r_sum / cov)))
                row_buf[idx + 1] = min(255, int(round(g_sum / cov)))
                row_buf[idx + 2] = min(255, int(round(b_sum / cov)))
                row_buf[idx + 3] = min(255, int(round(a_sum / samples)))
            else:
                row_buf[idx : idx + 4] = b"\x00\x00\x00\x00"
        out_rows.append(bytes(row_buf))
    return out_rows


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    assets = root / "assets"
    assets.mkdir(parents=True, exist_ok=True)
    iconset = assets / "AppIcon.iconset"
    if iconset.exists():
        shutil.rmtree(iconset)
    iconset.mkdir(parents=True)

    specs = [
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ]

    cache: dict[int, list[bytes]] = {}
    for filename, sz in specs:
        if sz not in cache:
            cache[sz] = render_icon(sz)
        write_png(iconset / filename, sz, sz, cache[sz])

    preview_path = assets / "AppIcon-1024.png"
    write_png(preview_path, 1024, 1024, cache[1024])

    icns_path = assets / "AppIcon.icns"
    subprocess.run(
        ["/usr/bin/iconutil", "-c", "icns", str(iconset), "-o", str(icns_path)],
        check=True,
    )
    shutil.rmtree(iconset)
    print(f"Generated {icns_path} and preview {preview_path}")


if __name__ == "__main__":
    main()
