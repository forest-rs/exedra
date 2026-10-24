# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Generate the directional UV diagnostic texture using only Python's stdlib."""

from pathlib import Path
import struct
import zlib


GLYPHS = {
    "A": [14, 17, 17, 31, 17, 17, 17],
    "B": [30, 17, 17, 30, 17, 17, 30],
    "C": [14, 17, 16, 16, 16, 17, 14],
    "D": [30, 17, 17, 17, 17, 17, 30],
    "1": [4, 12, 4, 4, 4, 4, 14],
    "2": [14, 17, 1, 2, 4, 8, 31],
    "3": [30, 1, 1, 14, 1, 1, 30],
    "4": [2, 6, 10, 18, 31, 2, 2],
}


def main():
    size = 512
    pixels = bytearray(size * size * 3)

    def rect(x, y, width, height, color):
        row = bytes(color) * width
        for pixel_y in range(y, y + height):
            start = (pixel_y * size + x) * 3
            pixels[start:start + width * 3] = row

    def text(x, y, value):
        for char in value:
            for row, bits in enumerate(GLYPHS[char]):
                for col in range(5):
                    if bits & (1 << (4 - col)):
                        rect(x + col * 5, y + row * 5, 5, 5, (26, 43, 55))
            x += 32

    colors = [(222, 231, 224), (170, 201, 205), (235, 197, 139), (210, 153, 123)]
    rect(0, 0, size, size, (35, 53, 66))
    for y in range(4):
        for x in range(4):
            px, py = x * 128, y * 128
            rect(px + 3, py + 3, 122, 122, colors[(x + y) % 4])
            text(px + 20, py + 36, f"{chr(65+y)}{x+1}")
            # Red arrow: increasing U. Green arrow: increasing V.
            rect(px + 20, py + 98, 77, 4, (173, 50, 48))
            rect(px + 105, py + 28, 4, 55, (22, 103, 88))
            for i in range(8):
                rect(px + 97 + i, py + 92 + i, 1, 16 - i * 2, (173, 50, 48))
                rect(px + 99 + i, py + 83 + i, 16 - i * 2, 1, (22, 103, 88))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    rows = b"".join(b"\0" + pixels[y * size * 3:(y + 1) * size * 3] for y in range(size))
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")
    output = Path(__file__).resolve().parents[1] / "assets" / "projection-grid.png"
    output.write_bytes(png)
    print(output)


if __name__ == "__main__":
    main()
