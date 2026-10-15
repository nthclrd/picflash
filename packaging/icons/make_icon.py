"""Draws the picflash app icon at 1024 px: a QFN chip crossed by a clock signal.

Drawn at 4x then downscaled for clean anti-aliased edges.
"""
import sys
from PIL import Image, ImageDraw, ImageFilter

K = 4
S = 1024 * K


def px(v):
    return int(v * K)


img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

# Tile: macOS grid (824 px body, 100 px margin), graphite to deep teal gradient.
tile = Image.new("RGBA", (S, S))
td = ImageDraw.Draw(tile)
top, bottom = (22, 27, 34), (10, 52, 60)
for y in range(S):
    t = y / S
    td.line([(0, y), (S, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,))
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle([px(100), px(100), px(924), px(924)], radius=px(185), fill=255)

# Soft drop shadow under the tile.
shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(shadow).rounded_rectangle([px(100), px(118), px(924), px(942)], radius=px(185), fill=(0, 0, 0, 110))
shadow = shadow.filter(ImageFilter.GaussianBlur(px(14)))
img.alpha_composite(shadow)
img.paste(tile, (0, 0), mask)

d = ImageDraw.Draw(img)
AMBER = (255, 184, 48, 255)
PIN = (158, 176, 186, 255)
BODY = (16, 20, 26, 255)

# Chip: pins on four sides, then the body on top.
body = [px(312), px(312), px(712), px(712)]
pins, pin_len, pin_w = 4, px(62), px(34)
span = body[2] - body[0]
for i in range(pins):
    c = body[0] + span * (i + 1) // (pins + 1)
    for box in (
        [c - pin_w // 2, body[1] - pin_len, c + pin_w // 2, body[1] + px(10)],
        [c - pin_w // 2, body[3] - px(10), c + pin_w // 2, body[3] + pin_len],
        [body[0] - pin_len, c - pin_w // 2, body[0] + px(10), c + pin_w // 2],
        [body[2] - px(10), c - pin_w // 2, body[2] + pin_len, c + pin_w // 2],
    ):
        d.rounded_rectangle(box, radius=px(9), fill=PIN)
d.rounded_rectangle(body, radius=px(46), fill=BODY, outline=(52, 64, 74, 255), width=px(8))
d.ellipse([body[0] + px(34), body[1] + px(34), body[0] + px(70), body[1] + px(70)], fill=(52, 64, 74, 255))

# Clock signal (square wave) on the chip, in amber.
y_hi, y_lo = px(452), px(572)
xs = [px(362), px(412), px(412), px(482), px(482), px(542), px(542), px(612), px(612), px(662)]
ys = [y_lo, y_lo, y_hi, y_hi, y_lo, y_lo, y_hi, y_hi, y_lo, y_lo]
d.line(list(zip(xs, ys)), fill=AMBER, width=px(40), joint="curve")
for x, y in ((xs[0], ys[0]), (xs[-1], ys[-1])):
    r = px(20)
    d.ellipse([x - r, y - r, x + r, y + r], fill=AMBER)

img.resize((1024, 1024), Image.LANCZOS).save(sys.argv[1])
