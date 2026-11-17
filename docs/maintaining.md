# Maintaining

## Layout

```
native/blitz            DioxusLabs/blitz at be60597 with vitrail's commits on branch vitrail-ui (a separate clone)
native/anyrender        DioxusLabs/anyrender at 812644a with vitrail's commits on branch vitrail-ui (a separate clone)
native/patches/*        the same commits as git patches (this repository keeps them; the clones are not in it)
native/vitrail-ffi        the native library (member of the blitz workspace)
native/setup.sh         re-creates native/blitz and native/anyrender from upstream and the patches (CI runs it)
native/build.sh         builds macOS (universal) and Windows (x86_64) into the package (package/Plugins) and the lab
package/                the Unity package com.vitrail.ui (the game uses it from here: Packages/manifest.json)
.github/workflows/      CI: macOS, Windows, Linux builds and tests, the package with all three libraries
tests/pages, tests/ref  test pages and their headless Chrome references (tests/chrome.sh: 2 s of virtual time, so scripts run)
tests/pages/vendor      petite-vue, Preact, htm (unchanged builds from npm, with their licenses) for the JavaScript tests
tests/gpu-suite.sh      every page in tests/suite.txt through the lab's GPU painter, compared with the CPU painter and Chrome
unity/                  the lab: a small Unity 6.3 URP project (Assets/Vitrail) for captures and experiments
docs/                   this documentation
```

## Building

- `native/build.sh` (both platforms) or `native/build.sh mac|win`. Windows is cross-compiled with mingw-w64
  (`brew install mingw-w64`, `rustup target add x86_64-pc-windows-gnu`).
- During development: `cargo build --release` in `native/vitrail-ffi`, then copy
  `native/blitz/target/release/libvitrail.dylib` into `unity/Assets/Vitrail/Plugins/macOS/`.
- The Unity editor reloads a rebuilt library by itself (`WebNative.Refresh`: it loads a copy, so the file is never
  locked).

## Re-creating the clones

`native/setup.sh` does this (and nothing for clones already there). By hand:

```sh
git clone https://github.com/DioxusLabs/blitz native/blitz && git -C native/blitz checkout -b vitrail-ui be60597
git -C native/blitz am ../patches/blitz/*.patch
git clone https://github.com/DioxusLabs/anyrender native/anyrender && git -C native/anyrender checkout -b vitrail-ui 812644a
git -C native/anyrender am ../patches/anyrender/*.patch
```

After committing a change in a clone, regenerate its patches:
`git -C native/blitz format-patch be60597 -o ../patches/blitz` (remove the old files first), likewise anyrender from
`812644a`.

## What the patches change

- **Blitz:** SVG paint from CSS, box-shadow order/radii/blur, text baseline snap and quarter-pixel glyphs, area-averaged
  image downscaling, gradient lengths on HiDPI, SVG clipped to its viewport; `inset()` round corners; texture
  placeholders never downscaled; backdrop-filter clipped to the rounded border box; compositor hooks (the composited set,
  `compositor_animations`, markers); CSS filter lengths scaled by the device scale; GPU scrolling; layout rounded to
  device pixels (rounding to whole CSS px at a fractional scale could round a 1 px border to nothing); form controls
  use Chrome's 13.333px font.
- **anyrender:** SVG filters through resvg (drop shadow in f32 linearRGB, box blurs, cached by what the group draws, never by its address: a freed tree's address is reused by the next page); chained filter bounds add
  up; composited element markers in recordings.

## Testing a change

1. `cargo test --release` in `native/vitrail-ffi`: behaviour (smoke, js, frameworks, forms, dev, fling), reuse drawing
   exactly what painting afresh draws (retained), the speed targets (perf, anim_budget; `PERF_SLACK` and `BUDGET`
   loosen them on slow machines), and on macOS `chrome.rs`: every suite page with a Chrome reference painted by the
   CPU painter must stay within 0.3/255 of `tests/chrome-baseline.txt` (`CHROME_BASELINE=write` records it after an
   intended change). The game HUD pages (top bar, turn panel) are in
   `tests/pages/game-hud`.
2. `tests/gpu-suite.sh <python with Pillow and numpy>`: every page's difference from the CPU painter and Chrome.
   Numbers that move without a reason are regressions.
3. In a Unity project: the sample pages look the same, `Stats` shows the costs, the console stays clean.
4. Rendering changes: measure (see [Performance](performance.md)) and add a line to the game's
   `docs/performance.md`.

## CI

Every push runs `.github/workflows/ci.yml`: on macOS 14, Windows 2022 and Ubuntu 22.04 it re-creates the clones
(`native/setup.sh`), runs the tests (Windows: the ones with pages written inline) and builds the library (macOS
universal, Windows with MSVC, Linux with its fontconfig); a last job puts the three into `package/Plugins` and
uploads `com.vitrail.ui-<version>.tgz`. The GPU painter's comparison (`tests/gpu-suite.sh`) needs Unity and
runs locally only.

Adding a feature: a test page in `tests/pages`, its Chrome reference (`tests/chrome.sh page.html ref/page.png w h`), a
line in `tests/suite.txt`, and a row in [CSS support](css-support.md).
