# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render matched cap wireframes from actual worker triangle buffers (requires Pillow)."""

import argparse
from collections import Counter
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
from compare import triangle_shape
from PIL import Image, ImageDraw, ImageFont


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("occt", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--case", choices=["plate_64"], default="plate_64")
    parser.add_argument("--native-tolerance", type=float, choices=[0.01, 0.02], default=0.02,
                        help="after-capture tolerance, used to label accuracy sensitivity figures")
    parser.add_argument("--font", type=Path, default=Path("/System/Library/Fonts/Helvetica.ttc"))
    args = parser.parse_args()
    scale = 3
    width, height = 2100, 1380
    image = Image.new("RGB", (width * scale, height * scale), "#f6f7f9")
    draw = ImageDraw.Draw(image)

    def text(x, y, value, size=23, fill="#23313e"):
        draw.text((int(x*scale), int(y*scale)), value,
                  font=ImageFont.truetype(str(args.font), size*scale), fill=fill)

    def line(points, fill, weight):
        draw.line([(round(x*scale), round(y*scale)) for x, y in points],
                  fill=fill, width=max(1, round(weight*scale)))

    text(62, 38, "64-hole plate  /  cap triangulation", 43)
    targets = "shared 0.02 mm linear target" if args.native_tolerance == 0.02 else (
        f"native before 0.02 mm · native after {args.native_tolerance:g} mm · OCCT 0.02 mm")
    text(64, 99, f"Actual mesh edges · 136 × 40 × 5 mm · {targets} · OCCT angle 3 rad", 23)
    titles = ["Native before", "Native after", "build123d / OCCT"]
    colors = ["#a45b4a", "#176c70", "#466692"]
    inputs = [args.before, args.after, args.occt]
    for column, (path, title, color) in enumerate(zip(inputs, titles, colors)):
        capture = json.loads(path.read_text())
        case = next(case for case in capture["cases"] if case["id"] == args.case)
        positions, triangles = case["positions"], case["triangles"]
        z = max(p[2] for p in positions)
        cap = [t for t in triangles if all(abs(positions[i][2] - z) < 1e-8 for i in t)]
        edges = Counter(tuple(sorted((a, b))) for t in cap for a, b in zip(t, t[1:] + t[:1]))
        left = 60 + column * 680
        text(left, 164, title, 34, color)
        text(left, 214, f"{len(triangles):,} total triangles · {len(cap):,} on this cap", 22)
        quality = triangle_shape(case, planar_xy_only=True)
        text(left, 249, f"Worst cap angle  {quality['minimum_angle_degrees']:.3f}°", 24, color)

        def panel(bounds, box):
            x0, x1, y0, y1 = bounds
            bx, by, bw, bh = box
            # Each panel is clipped separately, so the close-up includes edges
            # whose endpoints lie outside the visible window.
            tile = Image.new("RGB", (round(bw*scale), round(bh*scale)), "white")
            painter = ImageDraw.Draw(tile)
            factor = min(bw/(x1-x0), bh/(y1-y0))
            ox, oy = (bw-factor*(x1-x0))/2, (bh-factor*(y1-y0))/2
            def project(index):
                x, y, _ = positions[index]
                return ((ox+(x-x0)*factor)*scale, (oy+(y1-y)*factor)*scale)
            for (a, b), count in sorted(edges.items(), key=lambda item: item[1] == 1):
                painter.line([project(a), project(b)], fill="#273d4e" if count == 1 else color,
                             width=round((1.45 if count == 1 else 0.65)*scale))
            image.paste(tile, (round(bx*scale), round(by*scale)))
            draw.rectangle((round(bx*scale), round(by*scale), round((bx+bw)*scale), round((by+bh)*scale)),
                           outline="#d4dce3", width=scale)

        panel((-70, 70, -23, 23), (left, 307, 620, 218))
        text(left, 561, "Matching detail", 26)
        panel((-10, 14, -16, 8), (left, 610, 620, 620))
    if args.native_tolerance == 0.02:
        text(63, 1270, "The native update preserves boundary samples and surface error; the cap diagonals improve.", 25)
        text(63, 1310, "OCCT uses more boundary samples and has lower sampled surface error at the same nominal tolerance.", 23)
    else:
        text(63, 1270, "With the tighter native target, both updated meshes have 7,596 triangles and a 0.281° worst angle.", 25)
        text(63, 1310, "Both have approximately 0.00943 mm sampled surface error. This is a survey, not a global error certificate.", 23)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    image.resize((width, height), Image.Resampling.LANCZOS).save(args.output)
    print(args.output)


if __name__ == "__main__":
    main()
