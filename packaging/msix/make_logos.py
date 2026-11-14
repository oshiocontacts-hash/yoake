import argparse
from pathlib import Path

from PIL import Image


def main():
    parser = argparse.ArgumentParser(description="Generate the MSIX logo assets from the Yoake icon.")
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    arguments = parser.parse_args()

    arguments.output.mkdir(parents=True, exist_ok=True)
    with Image.open(arguments.source) as source:
        icon = source.convert("RGBA")
        for name, size in (
            ("Square44x44Logo.png", (44, 44)),
            ("Square150x150Logo.png", (150, 150)),
            ("Wide310x150Logo.png", (310, 150)),
            ("StoreLogo.png", (50, 50)),
        ):
            resized = icon.copy()
            resized.thumbnail((int(size[0] * 0.8), int(size[1] * 0.8)), Image.Resampling.LANCZOS)
            canvas = Image.new("RGBA", size, (0, 0, 0, 0))
            canvas.alpha_composite(resized, ((size[0] - resized.width) // 2, (size[1] - resized.height) // 2))
            canvas.save(arguments.output / name)


if __name__ == "__main__":
    main()
