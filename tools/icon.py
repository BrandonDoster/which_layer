# Black-background JPG -> assets/icon.png (transparent, cropped, 1024 square) + assets/icon.ico.
# From fang's tools/key_icon.py. Glow-on-black art: alpha = brightness, color un-premultiplied,
# so the glow fades into alpha; everything inside the outline is kept opaque.
# uv run --with pillow --with numpy --with scipy tools/icon.py <source.jpg>
import sys
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import binary_closing, binary_erosion, binary_fill_holes

assets = Path(__file__).resolve().parent.parent / "assets"
rgb = np.asarray(Image.open(sys.argv[1]).convert("RGB")).astype(np.float32) / 255
lum = rgb.max(axis=2)
floor = 0.06  # JPG noise in the "black"
a = np.clip((lum - floor) / (1 - floor), 0, 1)
color = np.where(a[..., None] > 0, np.clip(rgb / np.maximum(lum, 1e-6)[..., None], 0, 1), 0)
seal = 3
solid = binary_fill_holes(binary_closing(np.pad(lum > 0.35, seal), iterations=seal))[seal:-seal, seal:-seal]
# The keycap's dark lower body (~0.32) misses that cut and would speckle: also keep the whole
# silhouette solid, shrunk by the glow's width (~40 px at 2048) so the halo stays soft.
glow = max(1, round(40 * rgb.shape[0] / 2048))
solid |= binary_erosion(binary_fill_holes(lum > 0.1), iterations=glow)
a[solid] = 1
color[solid] = rgb[solid]
img = Image.fromarray((np.dstack([color, a]) * 255).round().astype(np.uint8), "RGBA")

ys, xs = np.nonzero(a > 0.02)
img = img.crop((xs.min(), ys.min(), xs.max() + 1, ys.max() + 1))
side = max(img.size)
canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
canvas.paste(img, ((side - img.width) // 2, (side - img.height) // 2))
canvas = canvas.resize((1024, 1024), Image.LANCZOS)
canvas.save(assets / "icon.png")
# Explorer/taskbar sizes at 100-250% scaling, plus 256 for large views.
canvas.save(assets / "icon.ico", sizes=[(s, s) for s in (16, 20, 24, 32, 40, 48, 64, 256)])
print("wrote", assets / "icon.png", assets / "icon.ico")
