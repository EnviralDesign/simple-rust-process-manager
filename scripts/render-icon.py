"""Renders the app icon from the mark the header paints (`paint_mark` in src/ui/header.rs).

Writes assets/icon.png (Windows and the window icon: the tile fills the canvas) and
assets/icon-macos.png (the tile on Apple's icon grid, 824 of 1024 with a soft shadow).
Run from the repository root: uv run --with pillow python scripts/render-icon.py
"""

from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024
SUPER = 4  # drawn at 4x and scaled down for smooth edges

PANEL2 = (0x16, 0x19, 0x1D, 255)
BORDER = (0x2F, 0x35, 0x3E, 255)
TEXT = (0xEC, 0xEE, 0xF2, 255)
GREEN = (0x5F, 0xBF, 0x62, 255)
AMBER = (0xE3, 0xA6, 0x3A, 255)
RED = (0xE2, 0x5F, 0x5F, 255)


def chevron(points, half):
    """The outline of a two-segment stroke `half` wide each side, with a mitred join and butt
    ends, as egui strokes the prompt."""
    (ax, ay), (bx, by), (cx, cy) = points

    def normal(x0, y0, x1, y1):
        dx, dy = x1 - x0, y1 - y0
        length = (dx * dx + dy * dy) ** 0.5
        return -dy / length, dx / length

    n1 = normal(ax, ay, bx, by)
    n2 = normal(bx, by, cx, cy)
    mx, my = n1[0] + n2[0], n1[1] + n2[1]
    scale = half / (mx * n1[0] + my * n1[1])  # the miter's length along the bisector
    mx, my = mx * scale, my * scale
    return [
        (ax + n1[0] * half, ay + n1[1] * half),
        (bx + mx, by + my),
        (cx + n2[0] * half, cy + n2[1] * half),
        (cx - n2[0] * half, cy - n2[1] * half),
        (bx - mx, by - my),
        (ax - n1[0] * half, ay - n1[1] * half),
    ]


def mark(tile: int) -> Image.Image:
    """The mark on a transparent square of `tile` pixels, on its 30-unit grid."""
    px = tile * SUPER
    s = px / 30.0
    image = Image.new("RGBA", (px, px), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    def p(x, y):
        return (x * s, y * s)

    def pill(x, y, w, h, color):
        draw.rounded_rectangle([p(x, y), p(x + w, y + h)], radius=min(w, h) / 2 * s, fill=color)

    # A thinner edge than the header's 1 point: at icon sizes a full unit reads as a frame.
    draw.rounded_rectangle(
        [p(0.5, 0.5), p(29.5, 29.5)], radius=7 * s, fill=PANEL2, outline=BORDER, width=round(0.6 * s)
    )
    draw.polygon([p(x, y) for x, y in chevron([(8, 10.5), (11.5, 13.5), (8, 16.5)], 1.0)], fill=TEXT)
    pill(14, 12.5, 8, 2, GREEN)
    pill(8, 19.5, 2, 2, AMBER)
    pill(12, 19.5, 10, 2, RED)
    return image.resize((tile, tile), Image.LANCZOS)


def windows() -> Image.Image:
    return mark(SIZE)


def macos() -> Image.Image:
    body = 824
    tile = round(body * 30 / 29)  # the grid's tile spans 29 of its 30 units
    canvas = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    art = mark(tile)
    offset = (SIZE - tile) // 2

    shadow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    alpha = art.split()[3].point(lambda a: a * 0.45)
    shadow.paste((0, 0, 0, 255), (offset, offset + 12), alpha)
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(14)))
    canvas.alpha_composite(art, (offset, offset))
    return canvas


if __name__ == "__main__":
    windows().save("assets/icon.png")
    macos().save("assets/icon-macos.png")
