"""Check the manifest's asset references before packing an MSIX without a PRI."""

import argparse
from pathlib import Path, PureWindowsPath
import xml.etree.ElementTree as ET


def asset_paths(manifest):
    paths = set()
    for element in ET.parse(manifest).iter():
        for value in [element.text or "", *element.attrib.values()]:
            path = PureWindowsPath(value.strip())
            if path.parts and path.parts[0].lower() == "assets":
                if path.is_absolute() or ".." in path.parts:
                    raise ValueError(f"Invalid manifest asset path: {value}")
                paths.add(path)
    if not paths:
        raise ValueError("The manifest contains no Assets references")
    return sorted(paths)


def check_assets(manifest, assets):
    missing = []
    for reference in asset_paths(manifest):
        path = Path(assets).joinpath(*reference.parts[1:])
        if not path.is_file() or path.stat().st_size == 0:
            missing.append(str(reference))
    return missing


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("assets", type=Path)
    args = parser.parse_args()
    missing = check_assets(args.manifest, args.assets)
    if missing:
        raise SystemExit("Missing or empty MSIX assets: " + ", ".join(missing))
    print(f"All {len(asset_paths(args.manifest))} manifest assets exist")


if __name__ == "__main__":
    main()
