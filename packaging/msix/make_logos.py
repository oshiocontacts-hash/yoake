import argparse
from pathlib import Path

from PIL import Image

# The Yoake icon is a pixel-art sun on a rounded plate, drawn on a grid of about 11.5 source pixels
# per dot (measured from its colour edges: they fall at multiples of ~11.5 px, so there is no whole
# number of source pixels per dot). Nothing is smoothed: the icon is only ever
#   - enlarged by a WHOLE number (1x, 2x, 4x ...) with nearest neighbour, the rest is margin; or
#   - shrunk with nearest neighbour when even 1x is too big for the logo.
# There are no fractional enlargements.
SOURCE_SIZE = 256

# Base size of each manifest logo; Windows asks for it at the scales below
# (Name.scale-100.png ... Name.scale-400.png). Pixel size = base size * scale / 100.
LOGOS = {
    "Square44x44Logo": (44, 44),
    "Square71x71Logo": (71, 71),
    "Square150x150Logo": (150, 150),
    "Wide310x150Logo": (310, 150),
    "Square310x310Logo": (310, 310),
    "StoreLogo": (50, 50),
    "SplashScreen": (620, 300),
}
SCALES = (100, 150, 200, 400)

# Wide tile and splash screen are filled with the dawn sky, in BANDS steps like the pixel art.
# Square tiles, the Store logo and the app-list icons stay transparent: the icon already has its
# own rounded plate, and Microsoft asks for transparent (unplated) app-list icons.
FILLED = ("Wide310x150Logo", "SplashScreen")
BANDS = 6
# The colour of the middle band; AppxManifest.xml.in uses it as the splash BackgroundColor.
SPLASH_BACKGROUND = "#34224d"

# Square44x44Logo is also the app-list icon: Windows looks for exact target sizes, and the
# "unplated" forms stop it from putting a plate behind the icon (taskbar, Start, dark and light).
TARGET_SIZES = (16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256)
TARGET_FORMS = ("", "_altform-unplated", "_altform-lightunplated")

LARGEST_FILL = 0.9  # an enlarged icon may take up to this much of the shorter side
SHRUNK_FILL = 0.8  # a shrunk icon takes this much (1.0 for the app-list icons)


def logo_files():
    """Every file this script writes: {file name: (width, height, logo name, filled)}."""
    files = {}
    for name, (width, height) in LOGOS.items():
        for scale in SCALES:
            size = ((width * scale + 50) // 100, (height * scale + 50) // 100)
            files[f"{name}.scale-{scale}.png"] = (*size, name, name in FILLED)
    for size in TARGET_SIZES:
        for form in TARGET_FORMS:
            files[f"Square44x44Logo.targetsize-{size}{form}.png"] = (size, size, "target", False)
    return files


def icon_side(short_side, shrunk_fill):
    """(drawn side in pixels, whole-number enlargement or None when shrunk)."""
    whole = int(short_side * LARGEST_FILL) // SOURCE_SIZE
    if whole >= 1:
        return SOURCE_SIZE * whole, whole
    return max(1, int(short_side * shrunk_fill)), None


def sky_bands(icon):
    """BANDS colours, top to bottom, from the icon's own sky (a column left of the sun)."""
    x = icon.width // 8
    top, bottom = icon.height // 12, icon.height - icon.height // 12
    colours = []
    for band in range(BANDS):
        y = top + (bottom - top) * (2 * band + 1) // (2 * BANDS)
        colours.append(icon.getpixel((x, y))[:3])
    return colours


def hex_colour(rgb):
    return "#%02x%02x%02x" % rgb


def render(icon, size, shrunk_fill, bands):
    width, height = size
    side, _ = icon_side(min(width, height), shrunk_fill)
    drawn = icon.resize((side, side), Image.Resampling.NEAREST)
    canvas = Image.new("RGBA", size, (0, 0, 0, 0))
    if bands:
        for index, colour in enumerate(bands):
            top = height * index // len(bands)
            bottom = height * (index + 1) // len(bands)
            canvas.paste((*colour, 255), (0, top, width, bottom))
    canvas.alpha_composite(drawn, ((width - side) // 2, (height - side) // 2))
    return canvas


def main():
    parser = argparse.ArgumentParser(description="Generate the MSIX logo assets from the Yoake icon.")
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    arguments = parser.parse_args()

    arguments.output.mkdir(parents=True, exist_ok=True)
    with Image.open(arguments.source) as source:
        icon = source.convert("RGBA")
        if icon.size != (SOURCE_SIZE, SOURCE_SIZE):
            raise SystemExit(f"expected a {SOURCE_SIZE}px square icon, got {icon.size}")
        bands = sky_bands(icon)
        middle = hex_colour(bands[BANDS // 2])
        if middle != SPLASH_BACKGROUND:
            raise SystemExit(f"SPLASH_BACKGROUND is {SPLASH_BACKGROUND}, the icon gives {middle}")
        for file, (width, height, name, filled) in logo_files().items():
            fill = 1.0 if name == "target" else SHRUNK_FILL
            render(icon, (width, height), fill, bands if filled else None).save(arguments.output / file)


if __name__ == "__main__":
    main()
