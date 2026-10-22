#!/usr/bin/env python3
"""Rasterize our simple SVG geometry into the native bundle icon formats.

Requires Pillow only for maintainers regenerating icons; builds use committed files.
"""
from pathlib import Path
import xml.etree.ElementTree as ET

from PIL import Image, ImageDraw

root = Path(__file__).resolve().parent / "src-tauri" / "icons"
source = ET.parse(root / "icon.svg").getroot()
size = 1024
scale = size / float(source.attrib["viewBox"].split()[2])
image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
draw = ImageDraw.Draw(image)
for node in source:
    tag = node.tag.rsplit("}", 1)[-1]
    if tag == "rect":
        draw.rounded_rectangle(
            (0, 0, float(node.attrib["width"]) * scale - 1,
             float(node.attrib["height"]) * scale - 1),
            radius=float(node.attrib["rx"]) * scale,
            fill=node.attrib["fill"],
        )
    elif tag == "polygon":
        points = [tuple(float(value) * scale for value in point.split(","))
                  for point in node.attrib["points"].split()]
        draw.polygon(points, fill=node.attrib["fill"])
for filename, width in [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)]:
    image.resize((width, width), Image.Resampling.LANCZOS).save(root / filename, optimize=True)
image.save(root / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
image.save(root / "icon.icns")
print("Generated PNG, ICO and ICNS icons from icon.svg")
