"""The MSIX manifest must resolve against the generated, PRI-free assets."""

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

try:
    from PIL import Image
except ImportError:
    Image = None

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("check_assets", ROOT / "packaging/msix/check_assets.py")
check_assets = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_assets)


@unittest.skipIf(Image is None, "Pillow is required to generate the MSIX logos")
class MsixAssetsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temp.cleanup)
        cls.assets = Path(cls.temp.name) / "Assets"
        cls.manifest = ROOT / "packaging/msix/AppxManifest.xml.in"
        subprocess.run(
            [sys.executable, str(ROOT / "packaging/msix/make_logos.py"),
             str(ROOT / "packaging/icons/yoake-256.png"), str(cls.assets)],
            check=True, capture_output=True, text=True,
        )

    def test_manifest_assets_match_scale_100_and_keep_qualified_variants(self):
        self.assertEqual(check_assets.check_assets(self.manifest, self.assets), [])
        for reference in check_assets.asset_paths(self.manifest):
            plain = self.assets / reference.name
            qualified = plain.with_name(f"{plain.stem}.scale-100.png")
            self.assertEqual(plain.read_bytes(), qualified.read_bytes(), str(reference))
        self.assertEqual(len(list(self.assets.glob("*.scale-*.png"))), 28)
        self.assertEqual(len(list(self.assets.glob("*.targetsize-*.png"))), 42)
        with Image.open(self.assets / "SplashScreen.png") as image:
            self.assertEqual(image.size, (620, 300))

    def test_missing_text_and_attribute_references_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            assets = Path(temporary)
            missing = check_assets.check_assets(self.manifest, assets)
            self.assertIn(r"Assets\StoreLogo.png", missing)
            self.assertIn(r"Assets\Square44x44Logo.png", missing)
            result = subprocess.run(
                [sys.executable, str(ROOT / "packaging/msix/check_assets.py"),
                 str(self.manifest), str(assets)], capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Missing or empty MSIX assets", result.stderr)

    def test_empty_asset_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            assets = Path(temporary)
            for reference in check_assets.asset_paths(self.manifest):
                (assets / reference.name).touch()
            self.assertEqual(len(check_assets.check_assets(self.manifest, assets)), 7)


if __name__ == "__main__":
    unittest.main()
