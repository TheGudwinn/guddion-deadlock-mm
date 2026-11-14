"""Renders the installer bitmaps (NSIS needs 24-bit BMP) in the app's night-glass style.

    python scripts/make-installer-art.py

Output: src-tauri/installer/sidebar.bmp (164x314) and header.bmp (150x57).
Fonts come from the self-hosted @fontsource packages, so run `pnpm install` first.
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
FONTS = ROOT / "node_modules/@fontsource"
OUT = ROOT / "src-tauri/installer"
S = 4  # supersampling factor

BG = (8, 9, 24)
VIOLET = (161, 147, 255)
DEEP = (106, 79, 230)
TEXT = (238, 236, 248)
MUTED = (163, 160, 189)


def font(pkg: str, name: str, size: int) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(str(FONTS / pkg / "files" / name), size * S)


def light(w: int, h: int) -> Image.Image:
    """Dark field with the same violet glow and diagonal streak as the app."""
    base = Image.new("RGB", (w * S, h * S), BG)
    glow = Image.new("RGBA", base.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(glow)
    d.ellipse([-w * S * 0.6, -h * S * 0.35, w * S * 1.1, h * S * 0.55], fill=(92, 56, 200, 150))
    d.polygon([(w * S * 0.15, 0), (w * S * 0.55, 0), (w * S * 1.0, h * S), (w * S * 0.45, h * S)], fill=(122, 92, 255, 55))
    glow = glow.filter(ImageFilter.GaussianBlur(26 * S))
    base.paste(glow, (0, 0), glow)
    return base


GEM = Image.open(ROOT / "assets/icon-source.png").convert("RGBA")


def diamond(img: Image.Image, cx: float, cy: float, size: float) -> None:
    """Pastes the app icon (assets/icon-source.png) centred at (cx, cy), `size` px wide."""
    g = GEM.resize((int(size * S), int(size * S)), Image.LANCZOS)
    img.paste(g, (int(cx * S - g.width / 2), int(cy * S - g.height / 2)), g)


def centered(draw: ImageDraw.ImageDraw, cx: float, y: float, text: str, f: ImageFont.FreeTypeFont, fill) -> None:
    w = draw.textlength(text, font=f)
    draw.text((cx * S - w / 2, y * S), text, font=f, fill=fill)


def sidebar() -> Image.Image:
    w, h = 164, 314
    img = light(w, h)
    diamond(img, w / 2, 96, 88)
    d = ImageDraw.Draw(img)
    centered(d, w / 2, 148, "Guddion", font("jost", "jost-latin-600-normal.woff", 27), TEXT)
    centered(d, w / 2, 186, "Deadlock mod manager", font("golos-text", "golos-text-latin-400-normal.woff", 10), MUTED)
    d.line([(w * S * 0.3, 214 * S), (w * S * 0.7, 214 * S)], fill=DEEP, width=2 * S)
    return img.resize((w, h), Image.LANCZOS)


def header() -> Image.Image:
    w, h = 150, 57
    img = light(w, h)
    diamond(img, 26, h / 2, 38)
    d = ImageDraw.Draw(img)
    d.text((48 * S, 14 * S), "Guddion", font=font("jost", "jost-latin-600-normal.woff", 18), fill=TEXT)
    d.text((48 * S, 35 * S), "mod manager", font=font("golos-text", "golos-text-latin-400-normal.woff", 8), fill=MUTED)
    return img.resize((w, h), Image.LANCZOS)


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    sidebar().convert("RGB").save(OUT / "sidebar.bmp", "BMP")
    header().convert("RGB").save(OUT / "header.bmp", "BMP")
    print("wrote", OUT / "sidebar.bmp", OUT / "header.bmp")
