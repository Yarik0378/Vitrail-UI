"""Compare two RGBA renders: error over black and over white (so alpha counts), a heat map, and a side-by-side sheet.
Usage: diff.py ref.png ours.png sheet.png"""
import sys
import numpy as np
from PIL import Image

def load(p):
    return np.asarray(Image.open(p).convert("RGBA")).astype(np.float64) / 255

a, b = load(sys.argv[1]), load(sys.argv[2])
h, w = min(a.shape[0], b.shape[0]), min(a.shape[1], b.shape[1])
a, b = a[:h, :w], b[:h, :w]
def over(x, bg): return x[..., :3] * x[..., 3:] + bg * (1 - x[..., 3:])
errs = []
for bg in (0.0, 1.0):
    errs.append(np.abs(over(a, bg) - over(b, bg)) * 255)
e = np.maximum(errs[0], errs[1]).max(axis=2)
mean, p99, mx = e.mean(), np.percentile(e, 99), e.max()
bad = (e > 8).mean() * 100
print(f"mean {mean:.2f}/255  p99 {p99:.1f}  max {mx:.0f}  >8: {bad:.2f}% px")
def show(x):
    chk = ((np.indices((h, w)).sum(0) // 8) % 2)[..., None] * 0.15 + 0.35
    return (over(x, chk) * 255).astype(np.uint8)
heat = np.clip(e * 8, 0, 255).astype(np.uint8)
heat = np.stack([heat, heat // 3, np.zeros_like(heat)], -1)
sheet = np.concatenate([show(a), show(b), heat], axis=0)
Image.fromarray(sheet).save(sys.argv[3])
