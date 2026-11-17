# Architecture

## Pieces

```
HTML + CSS ──► Blitz (Rust)                         ──► GPU frame        ──► Unity (C#, shaders)
               Stylo: styles, animations                 triangles            WebGpuPainter: draws the frame
               Taffy: layout                             batches + ops        WebFilters: filters, glass, layers
               Parley: text                              filters, layers      WebPanel: page on screen, input
               blitz-paint: draw commands  ──► vitrail-ffi: gpu.rs
```

- `native/blitz`, `native/anyrender`: DioxusLabs' Blitz and anyrender with vitrail's patches (`native/patches`).
- `native/vitrail-ffi`: the native library (`vitrail.dll`, `libvitrail.dylib`): a C API (`src/lib.rs`), the GPU frame
  builder (`src/gpu.rs`), the CPU painter's tiles (`src/tiles.rs`).
- Unity side (the package `package/`, namespace `Vitrail`): `WebNative` (bindings, hot
  reload of the library in the editor), `WebPanel` (a page on any rectangle of the screen, `Area`; several panels
  draw in one frame, each compositing through its own material), `WebGpuPainter`, `WebFilters`, shaders
  `WebGpu.shader`, `WebFilter.shader`, `WebComposite.shader`.

## A frame's life

1. The game changes something: `SetText`, `SetClass`, `SetStyle`, the pointer moves (`:hover`), the clock ticks
   (`vitrail_tick`), the page file is saved. The page is marked dirty. Nothing else happens until then: an idle page costs
   nothing but its compositing.
2. Main thread, `vitrail_paint_start` (≈0.3 ms at 4K): Stylo restyles what changed, Taffy lays out, blitz-paint records
   the draw commands.
3. Worker thread: the commands become the GPU frame (`gpu::build`, ≈0.2–0.4 ms): fills and strokes tessellated with
   soft edges, glyph quads from the atlas, gradient ramps, images, clips and groups, filter steps.
4. Main thread, `vitrail_paint_poll` hands the frame over; `WebGpuPainter.Paint` uploads it and records the drawing.
5. Every camera frame, `WebPanel`'s render pass composites the page texture over the scene (browser blending: in sRGB,
   premultiplied), with glass drawn inside the frame when there is any. The pass runs after post-processing, where
   URP's picture still has the render scale's size: when that differs from the screen, the first page's pass brings
   the scene to the screen's size (one smoothed blit) and makes it the camera's colour, so pages land pixel for pixel
   (drawn into a 150% picture they were resampled with it and came out blurred). FXAA is the exception the pass
   can't help: URP runs it in its final pass, after the pages, and it softens their text; SMAA and MSAA don't.

## The GPU frame

One vertex buffer (60 bytes: position, premultiplied sRGB colour carrying edge coverage, uv, 4 parameters, the
material and up to three shader clips, the place in the image atlas), one index buffer, batches and a list of ops.

**One draw for runs of anything.** Every draw uses the uber material: the vertex says whether it is a fill, a gradient,
text, an image, a box shadow or an inset shadow, and which shader clips cut it; small images that clamp at their edges
are packed into a 2048² image atlas. So a batch breaks only where state really changes (a group, a stencil clip, a big
or repeating image). A 2000-cell inventory (250 cells on screen) is one draw call; it was 1471.

Ops:

| Op | Meaning |
|---|---|
| 0 | draw a batch |
| 1 / 2 | push / pop a stencil clip (axis-aligned rectangles) |
| 3 / 4 | push / pop a group: an offscreen texture, composited through its soft clip with opacity, a Porter-Duff operator or a blend mode, after its filter steps |
| 5 | backdrop-filter: the scene and the page so far, filtered, put back under the element |
| 6 / 7 | a composited layer starts / ends |

**Antialiasing** is analytic: each outline is drawn inset by half a pixel at full coverage plus a one-pixel fringe
fading to zero (as NanoVG and Skia's GPU path do). No MSAA, so targets take an eighth of the memory.

**Clips without textures.** Rectangles, rounded rectangles (border-radius, circles, ellipses) and everything but one
(an outer box-shadow's clip) are applied per pixel by every shader from a clip table (a float texture, up to 16 383
clips a frame, `vitrail_gpu_clips`); each vertex names up to three (nested). Only when three are already open does a
rectangle fall back to the stencil. Inset box-shadows, which Blitz records as a layer
with a DestOut layer inside, are drawn in one pass (the colour where the hole doesn't reach). Only other clip shapes
(stars, paths) and true isolation (opacity, blend modes, filters) still need a group's offscreen texture: the top bar
went from 33 groups to 1 (its glass).

**Stable places.** The vertex buffer is a pool: each draw's vertices are found by their content and keep their place
across frames, a changed draw takes a freed block or the end, and the frame reports the spans it wrote
(`vitrail_gpu_vertex_spans`, `vitrail_gpu_vertex_range`). Gradient rows and clip rows are stable tables for the same
reason (a vertex naming them stays the same). The index buffer, which holds the draw order, is rebuilt each frame.

**Caches** keep work between frames: glyphs in an R8 atlas (rasterized once by the CPU painter, so text looks the
same), triangles of every shape cached at the origin by outline, linear transform and style (a moving shape is never
tessellated again), gradient ramps by their stops, colour glyphs as images.

## Retained painting

Blitz records each element's draw commands into its own shared piece (`RenderCommand::Group`, an `Arc`) and keeps it
with what it was painted for (its transform, cull rectangle, box, scroll offset and the scale). While painting, an
element that is not damaged and holds nothing damaged (`BaseDocument::paint_dirty`, gathered as `resolve` clears
damage, plus scrolls, composited-set changes and loaded backgrounds) appends its piece instead of walking its subtree;
a changed element re-records its own commands and appends its children's pieces. The frame builder walks pieces by
reference and keeps what each simple piece drew (vertex places and pool keys, indices, gradient and shader-clip rows)
under its address and context (open shader clips, scroll space, glyph atlas): an unchanged piece appends those
indices again. A piece holding a real step (a stencil clip, an offscreen group, a layer marker) is walked as before.

## Pieces, glass and the compositor

The painter cuts the frame at its top level (where only stencil clips are open) into pieces:

- **still** pieces are drawn once per change, each into its own texture;
- **glass** pieces (a backdrop-filter and the group around it) are drawn every frame inside the camera's render graph,
  after the scene band is copied, because the scene under them moves;
- **moving** pieces are composited layers.

Every frame the painter lays the pieces together in order (one textured draw per still piece). Piece and layer
textures cover only the page rectangle their content draws in.

**Dirty rectangles.** The frame reports up to four page rectangles that changed (`vitrail_gpu_dirty`: the boxes of the
draws written and freed, joined; the whole page when the frame's structure changed). A kept texture whose piece is the
same as last time is redrawn only inside them, through a scissor: a top-bar button change redraws 105×105 px of
3840×323. The top bar with its
glass costs 5 steps a frame instead of 125.

**Compositor.** Before a paint, elements whose running animations touch only `transform`, `translate`, `rotate`,
`scale` or `opacity` become composited: Blitz paints them between markers at full opacity, and the frame records them
as layers. While only such layers move, `vitrail_tick` returns 2 after a restyle (no layout, no paint, no frame build),
and `vitrail_layer_states` gives each layer the matrix from where it was painted to where it is now, and its opacity;
the painter draws the layer's texture with them (0.012 ms a frame). When the animations end, the page paints once more
so it is crisp as it now stands. An element inside a group (an opacity or a soft clip around it) is drawn with the page
instead, and its animation repaints.

## Scrolling on the GPU

In GPU mode Blitz paints a scroll container's content unscrolled between scroll markers, with 600 CSS px of extra
content beyond each side of the scrollport (`gpu_scroll_margin`). The content's vertices name their container
(`Vertex::meta`, top 8 bits) and the vertex shader moves them by the container's offset from a small table
(`_Scroll`, `vitrail_scroll_states`); clips name the scroll space they live in, so a cell's rounded clip moves with the
cell while the window's clip stays. `vitrail_scroll` returns 2 while the new offset stays within the held content: no
restyle, no paint, no new vertices; the painter only redraws its kept textures inside the container's box. Past the
margin it returns 1 and the page paints at the new offset.

## Overlay scrollbars

The panel draws the thumbs of GPU-scrolled containers itself, in the composite pass over the page, each through a
viewport over its box (a rounded rectangle in the shader): they show when a container scrolls or a thumb is hovered,
fade half a second later, and dragging one scrolls its container. Nothing of the page repaints while they show, move or
fade (`vitrail_scroll_extents` gives the sizes).

## Glass

backdrop-filter stays in the frame as op 5. The painter takes the target so far (the page under the element) over the
scene band (`_VitrailScene`, converted to sRGB), runs the filter steps within the element's bounds (the blur repeats edge
pixels, as Chrome does), and composites the result through the element's rounded shape. The element then draws over
it.

## JavaScript

`js.rs` runs the page's scripts on QuickJS (rquickjs). The DOM is written in JavaScript (`dom.js`) over about thirty
native operations on node ids (query, read and change nodes, attributes, styles and text, rectangles), so the native
side stays small and every node is one cached JS object. Each call into JavaScript (scripts, events, timers, the
game's calls) takes the runtime out of the page, points the native operations at it, and runs the promise jobs left
afterwards. Pointer events reach scripts only for types some listener waits for; timers run inside `vitrail_tick` on the
game's clock. `engine.trigger` queues events and `console` queues messages for the game to take each frame.

## Coordinates and colour

Device pixels, y down, set by `SetViewProjectionMatrices(identity, Ortho(0, w, h, 0))`. Shaders that read a texture
at another pixel map page pixels to texture coordinates through the per-pixel derivative of the page position, since
rendering into a texture may flip y. Colours are sRGB values, premultiplied, in UNORM targets: blending happens on
sRGB values as in a browser; the panel converts the scene to sRGB, mixes, and converts back.

## Errors

Every exported function catches panics (`guard`): it returns its fallback and keeps the message for
`vitrail_last_error`; `WebNative.ReportErrors` writes it to the Unity console. If the GPU painter can't start (its
shader is missing or unsupported), the CPU painter takes over and the panel says why. Before a document is destroyed
(a reload, a rebuilt library) the painter forgets it (`WebGpuPainter.Forget`): its per-frame glass recording once asked
the destroyed document for layer states and crashed the editor.
