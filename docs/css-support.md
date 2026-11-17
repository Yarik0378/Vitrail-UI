# CSS support

What vitrail draws, and how close it is to Chrome. "Mean" is the mean per-pixel difference against headless Chrome
(over black and over white, out of 255) on the test page named; under 1 is not visible. Pages live in `tests/pages`,
Chrome references in `tests/ref`, and `tests/gpu-suite.sh` re-measures everything.

Styles, selectors, the cascade, variables, `calc()` and layout come from Firefox's CSS engine (Stylo) and Taffy, so
they follow the specifications; the table below is about drawing.

## Supported

| Feature | Notes | Test page | Mean vs Chrome |
|---|---|---|---|
| Backgrounds, borders, `border-radius` (elliptical too) | | 01-box | 0.35 |
| `linear-`, `radial-`, `conic-gradient`, repeating ones | interpolated in sRGB with premultiplied alpha | 02-gradients | 0.42 |
| `box-shadow`, `inset` shadows | blurred rounded rectangles in a shader | 03-shadows | 0.80 |
| Text, `@font-face`, system fonts and fallback | glyphs cached in an atlas; Cyrillic, CJK | 04-text, 12-gamefont | 1.4–3 (font hinting) |
| Scaled and rotated text, very large text | scaled: rasterized at the shown size; rotated/huge: outlines | 26-text2 | — |
| Colour emoji | sbix/CBDT fonts (macOS, Linux); Windows COLR emoji not yet | 26-text2 | matches by eye |
| Inline SVG: paths, strokes, gradients, `filter` (drop shadow) | SVG filters rasterized by resvg | 05-svg, 10-filter | 0.41, 0.83 |
| `<img>`: PNG, JPEG, WebP, GIF, SVG, `data:` URLs | downscaled with area averaging | 16-icon | — |
| Game textures: `<img src="texture:name?w=&h=">` | any Unity texture (render textures too) set from C# | 25-texture | exact colours |
| `opacity`, nested layers | | 13-panel | 0.24 vs CPU painter |
| `filter`: blur, drop-shadow, grayscale, brightness, contrast, saturate, hue-rotate, sepia, invert, opacity, chains | colour matrices exact; lengths scale with the device scale | 22-cssfilter | 1.27, at 2× 1.00 (text) |
| `backdrop-filter` (glass) | blurs the game scene and the page under the element, in its rounded shape | 27-backdrop | 0.66 |
| `mix-blend-mode`: all 15 modes | | 23-blend | 0.83 |
| `clip-path`: circle, ellipse, polygon, inset (round), path | soft edges | 24-clipmask | 0.41 |
| `mask-image`, `mask` (gradients, repeating) | | 24-clipmask | 0.41 |
| `overflow: hidden` with `border-radius` | | 24-clipmask | 0.41 |
| `transform` (2D) | | 26-text2 | — |
| `transition`, `@keyframes` | transform/opacity run on the compositor; transitionrun/start/end/cancel and animationstart/iteration/end/cancel reach scripts | tests/animation.rs, js.rs | — |
| `el.animate()` (Web Animations) | keyframes (array or object form), duration, delay, iterations, direction, easing, fill; `finished`, `onfinish`, `pause`, `play`, `cancel`; played by the CSS animation engine (transform/opacity on the compositor) | js.rs | — |
| `:hover`, `:active`, `cursor` | the pointer comes from the game | | |
| Scrolling (`overflow: auto/scroll`), the mouse wheel | moved on the GPU; overlay scrollbars drawn by the GPU painter (shown while scrolling or hovered, fading, dragged, track clicks) | fling.rs, smoke.rs | — |
| `<select>`, `<option>`, `<optgroup>` | the closed box shows the chosen option (`option:checked`) and is as wide as the widest; open, a `<vitrail-popup>` of `<vitrail-option>`s (style `vitrail-popup`, `vitrail-option`, `vitrail-option[current]`, `[checked]`, `[disabled]`, `vitrail-optgroup`); mouse, keys (arrows, Home/End, PageUp/Down, Enter, Space, Escape, Alt+arrows, F4) and the gamepad; `input`/`change` events; `value`, `selectedIndex`, `options`, `option.selected` in scripts; no `multiple` or `size` lists yet | forms.rs | — (Chrome draws the native control) |
| `:focus-visible` | after the keyboard or gamepad moved the focus, and in text fields; not after a mouse press | forms.rs | — |
| Gamepad (spatial) navigation | `vitrail_navigate` moves the focus by direction between focusable elements (not `tabindex="-1"`), scrolling it into view; `vitrail_activate` presses it; `vitrail_cancel` backs out | forms.rs | — |
| HTML drag and drop (`draggable`, `dragstart`, `drag`, `dragenter`, `dragover`, `dragleave`, `drop`, `dragend`, `DataTransfer`) | for pages with scripts; a 70% copy of the source (or the `setDragImage` element) follows the pointer; no files from the system | forms.rs | — |
| JavaScript UI libraries: petite-vue, Preact (hooks, htm) | run unchanged; the DOM they build is drawn as Chrome draws it | 40-petite-vue, 41-preact | 0.88, 2.63 (button borders) |
| ES modules (`type="module"`), import maps, JSON imports, top-level await | modules are files next to the page; Chrome refuses modules from file: pages, so no reference | 42-preact-modules | — |

Pages compared only with vitrail's CPU painter (their Chrome references were taken at another scale): 06-topbar 0.37,
13-panel 0.24, 15-ring 0.06.

## Not yet

- `<select multiple>` and list boxes (`size` > 1). `scrollbar-width`, `scrollbar-color` and `::-webkit-scrollbar`
  styles aren't applied to the overlay scrollbars.
- 3D transforms (`perspective`, `rotateX`): drawn flat.
- Windows colour emoji (COLR fonts), `-webkit-text-stroke`.
- Scrollbars, `position: sticky` in scrolled areas (untested).
- Network URLs: only `file:`, `data:` and `texture:` load.

## Differences you may see

- Text: Chrome on macOS uses CoreText; vitrail uses its own rasterizer with CoreText-like stem darkening. Edges of glyphs
  differ by a few levels; shapes and positions match.
- Layout snaps to device pixels as Chrome does, so a 1 px border stays at least one device pixel at any scale.
- Rectangular clips off whole pixels are cut by the stencil (within half a pixel) rather than antialiased.
- The CPU painter (the fallback when the GPU painter can't start) draws no emoji, no CSS filter colour matrices and
  no game textures.
