import argparse
from pathlib import Path

from PIL import Image

# Base size of each manifest logo; Windows asks for it at the scale factors below
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

# Square44x44Logo is also the app-list icon: Windows looks for exact target sizes, and the
# "unplated" forms stop it from putting a plate behind the icon (taskbar, Start, dark and light).
TARGET_SIZES = (16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256)
TARGET_FORMS = ("", "_altform-unplated", "_altform-lightunplated")


def logo_files():
    """Every file this script writes, as {file name: (width, height)}."""
    files = {}
    for name, (width, height) in LOGOS.items():
        for scale in SCALES:
            files[f"{name}.scale-{scale}.png"] = ((width * scale + 50) // 100, (height * scale + 50) // 100)
    for size in TARGET_SIZES:
        for form in TARGET_FORMS:
            files[f"Square44x44Logo.targetsize-{size}{form}.png"] = (size, size)
    return files


def render(icon, size, fill):
    width, height = size
    resized = icon.copy()
    resized.thumbnail((max(1, int(width * fill)), max(1, int(height * fill))), Image.Resampling.LANCZOS)
    canvas = Image.new("RGBA", size, (0, 0, 0, 0))
    canvas.alpha_composite(resized, ((width - resized.width) // 2, (height - resized.height) // 2))
    return canvas


def main():
    parser = argparse.ArgumentParser(description="Generate the MSIX logo assets from the Yoake icon.")
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    arguments = parser.parse_args()

    arguments.output.mkdir(parents=True, exist_ok=True)
    with Image.open(arguments.source) as source:
        icon = source.convert("RGBA")
        for file, size in logo_files().items():
            # Target-size icons fill their square; tiles and splash leave a margin.
            fill = 1.0 if ".targetsize-" in file else 0.8
            render(icon, size, fill).save(arguments.output / file)


if __name__ == "__main__":
    main()
