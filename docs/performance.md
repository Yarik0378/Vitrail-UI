# Performance

Measured on an Apple M4, a game top bar at 4K (`tests/pages/game-hud/topbar.html`) (3840×323 device px, scale 2.15) unless said otherwise. The
benchmarks are in `native/vitrail-ffi/tests/bench.rs` (`cargo test --release --test bench -- --nocapture`); in Unity,
`WebPanel.Stats` shows the last change's cost.

## What a change costs

Measured by `tests/perf.rs` (`cargo test --release --test perf -- --nocapture`): the main thread counts the change
itself, styles and layout, and the draw list; the worker builds the GPU frame beside the game.

| Situation | Main thread | Worker | Target |
|---|---|---|---|
| The top bar, a hover | 0.067 ms | 0.14 ms | ≤ 0.3 ms main |
| A 2000-cell inventory, one cell changes | 0.155 ms (was 2.22) | 0.12 ms (was 1.6) | ≤ 0.3 ms main |
| A 4K window of 100 cards, a hover | 0.104 ms (was 0.54) | 0.19 ms (was 0.76) | ≤ 0.3 ms main, ≤ 3 ms in all |
| 20 cards changing colour, a frame | 0.148 ms (was 0.265) | 0.14 ms | ≤ 0.3 ms main |
| A transform/opacity animation, a frame | 0.012 ms (restyle + layer states), no paint | — | |
| Wheel steps on the inventory (40 px) | mean 0.21–0.24 ms; every step moves on the GPU, and the content is repainted ahead when half the margin is left: 2–2.4 ms main (was 8.3), cells reused | | mean ≤ 0.3 ms |
| First paint of a page | 50–150 ms (fonts, images, glyphs) | | |
| JavaScript: a page's scripts at load / a click and the library's update | petite-vue 2.2 ms / 0.043 ms, Preact 3.1 ms / 0.102 ms | | |

A change costs what it touches, not the page: the inventory's cell costs the same as a button of the top bar.

## On the GPU

After the uber pass and half-size glass (second measurement): the page 0.42–0.47 ms a frame (glass 0.34–0.38),
10 draw calls; a repaint ≈0.38 ms before dirty rectangles (it redrew both full-width still pieces), now only the
changed rectangles.

Measured by frame-time A/B in the Unity editor (Metal has no GPU timer there), 4K, the top bar idle: about 0.6–0.8 ms a
frame and 14 draw calls before the fixes below (glass ≈0.3–0.55 ms, scene band copy and composite ≈0.25–0.45 ms, the
still pieces ≈0.03 ms); one repaint ≈0.15 ms in its frame. Since then the glass blurs at half size when σ ≥ 6 px and
the composite draws only the page's band.

## Why it is fast

- **Only changes paint.** The page is dirty only after a setter, the hover, the clock or a resize; setters skip
  unchanged values.
- **Off the main thread.** Styles and layout need the document (main thread, a fraction of a millisecond); building
  the GPU frame runs on a worker, and the screen keeps the last frame until the new one is in.
- **Caches.** Glyphs rasterize once into an atlas; shapes tessellate once and are cached at the origin, so moving shapes
  reuse them; gradient ramps are kept by their stops; images upload once per id.
- **Antialiasing in the triangles.** No MSAA targets: an eighth of the memory, no resolve.
- **Uber material.** All materials in one pass and small images in an atlas: one draw call for long runs of mixed
  content (the inventory: 1471 → 1).
- **Clips in the shaders.** Rounded clips and inset shadows are computed per pixel, not through offscreen textures
  (top bar: 33 groups → 1, 30 000 → 13 500 vertices).
- **Pieces.** With glass or composited layers, the still parts of the page are drawn once into textures and laid
  together each frame with one draw per piece; each texture is only as big as what it holds (the top bar's sliding
  buttons: 423×107 instead of 3840×323).
- **Scrolling on the GPU.** Scroll containers hold their content unscrolled with a margin; a wheel step only changes
  an offset table (0.026 ms) until it leaves the held content.
- **Compositor.** transform/opacity animations move a layer texture: no layout, no paint, no new vertices.
- **Dirty rectangles.** A change redraws kept textures only where draws changed (up to four rectangles, scissored):
  a button 1% of the top bar, one inventory cell 0.8% of the page.
- **Scrolling reuses the cells.** The rectangle content is culled by moves in steps of half the margin, and an element
  wholly inside it is kept whatever the rectangle, so repainting a scrolled list reuses its items. Blitz's limit of
  1024 layers a frame is lifted (it made an element's painting depend on the elements before it, which reuse can't
  keep; the GPU painter clips per pixel and has no such limit).
- **Stable vertex places.** Each draw's vertices keep their place in the vertex buffer across frames (found by their
  content; gradient rows and clip rows are stable too), so a change uploads only the draws that changed: one cell of
  the inventory 124 vertices (0.5% of the buffer; it was 54–95%), a top-bar button 9–20 KB. Indices are rebuilt from
  their first difference (they are a sixth of the size).
- **Retained painting.** Blitz keeps each element's recorded draw commands as a shared piece and reuses it unless
  the element or something inside it was damaged since (scrolling, composited-layer changes and loaded backgrounds
  mark it too; text fields and selections are always repainted). Layout is skipped when nothing needs it (the root's
  cache survived), and a colour change no longer relays out. The frame builder replays an unchanged piece's draws
  (its vertex places, indices, gradient and clip rows) instead of walking its commands. Reuse is tested to draw
  exactly what painting afresh draws: pixels, commands and the GPU frame's triangles (`tests/retained.rs`).
- **Glass at a reduced size.** A backdrop blur runs at a quarter of the size from σ 6 px and a half from σ 3 px (as
  Skia does for big blurs), each texel averaging the pixels it covers; 27-backdrop against Chrome 0.66 → 0.76/255, not
  visible. The top bar's glass texture: 960×80 instead of 1920×161.

## What is still to do

1. (Done, see "Retained painting" below.) On big pages the CPU side dominated: a hover rebuilds the whole frame (≈3.3 ms for the inventory; restyle ≈0.7,
   draw list ≈1.5, the rest the frame build). The frame build already reuses everything unchanged (vertex slots found
   by a cheap key of the count and three vertices, confirmed by comparing bytes; clip shapes cached by outline and
   transform). The rest needs Blitz to keep each element's draw commands between paints and redo only the changed
   element's.
2. Merging the panel's passes into one native render pass (no store between the scene band copy and the composite)
   is not possible while there is glass: the glass must sample the scene band as it is this frame, which forces the
   attachment to be stored. It would only help pages without glass, so it waits.

## Measuring

- `cargo test --release --test bench -- --nocapture` prints the benchmarks; `VITRAIL_PROF=1` splits `vitrail_gpu_build`
  into restyle, draw list and frame build, with the number of shapes tessellated anew.
- `tests/gpu-suite.sh` prints each test page's build+draw time in the lab project's log.
- In Unity, `WebPanel.Stats`: main-thread and worker time of the last change (the worker figure includes
  waiting for the next game frame), and what is drawn every frame.
