# C API

The native library's interface (`native/vitrail-ffi/src/lib.rs`), for engine integrations. Strings are UTF-8 and
NUL-terminated. A page is a `Vitrail*`; all calls on one page must come from one thread (the game's main thread); the
library runs its own worker. Every function catches panics and returns its fallback (0, null, nothing);
`vitrail_last_error` tells what happened.

## Pages

| Function | Does |
|---|---|
| `Vitrail* vitrail_create(html, base_url, width, height, scale)` | A page from HTML text. `base_url` (a `file:` URL) resolves its links; size in device px; `scale`: device px per CSS px. Files load synchronously. |
| `void vitrail_resize(ui, width, height, scale)` | New size or scale. |
| `void vitrail_destroy(ui)` | Frees the page (waits for its worker). |
| `usize vitrail_last_error(buf, cap)` | The last panic's message (copied into `buf`, length returned), then cleared. |

URLs: `file:` and `data:` load; `texture:name?w=&h=` makes a transparent placeholder of that size which the GPU
painter replaces with a game texture (`vitrail_texture_id`, `vitrail_gpu_image_texture`).

## Changing the page

All return 1 when an element matched. A change marks the page to paint.

| Function | Does |
|---|---|
| `vitrail_set_text(ui, selector, text)` | Text content of the first match. |
| `vitrail_set_attr(ui, selector, name, value)` | An attribute. |
| `vitrail_set_class(ui, selector, class, on)` | Adds (on ≠ 0) or removes a class. |
| `vitrail_set_style(ui, selector, property, value)` | One inline CSS property. |
| `int vitrail_tick(ui, seconds)` | Advances the clock of CSS animations and transitions (any steady clock). Returns 0 when nothing animates, 1 when the page must paint, 2 when only composited layers move (no paint: read `vitrail_layer_states`). Call every frame. |

## JavaScript

| Function | Does |
|---|---|
| `int vitrail_js_call(ui, name, json)` | Calls the page's `engine.on(name)` handlers with `json` (an array of arguments, or one value). 0 when the page has no scripts. |
| `int vitrail_js_poll(ui, name, name_cap, json, json_cap)` | Takes the oldest `engine.trigger(name, ...args)`: name and arguments (JSON array). 1 taken, 0 none, −n a buffer needs n bytes (nothing taken). |
| `int vitrail_js_log(ui, buf, cap, *level)` | Takes the oldest console message or uncaught error; level 0 log, 1 warning, 2 error. Returns as `vitrail_js_poll`. |
| `int vitrail_js_eval(ui, source)` | Runs code in the page (starting an engine if it had none). |

| `int vitrail_key(ui, down, key, code, mods, repeat)` | A key down (1) or up (0), named as the DOM does (`"a"`, `"Enter"`; `"KeyA"`); `mods` 1 Shift 2 Ctrl 4 Alt 8 Meta. Scripts hear keydown/keyup on the focused element (or the body); unless prevented, a focused element gets the default (typing into a text field, Tab). Returns 1 when the page took the key. |
| `int vitrail_editing(ui)` | 1 while a text field has the focus (leave typing keys to the page). |
| `void vitrail_blur(ui)` | Takes the focus away (blur/focusout reach scripts). |
| `int vitrail_clipboard_take(ui, buf, cap)` | Text the page copied (Ctrl/Cmd+C or X in a field), for the system clipboard. 1 taken, 0 none, −n too small. |
| `int vitrail_paste_wanted(ui)` / `void vitrail_paste(ui, text)` | The page asked to paste (Ctrl/Cmd+V); the game answers with the clipboard's text. |
| `int vitrail_ime_state(ui, float out[4])` / `void vitrail_ime(ui, kind, text, caret)` | Whether a field wants the IME and where its caret is (device px); a composition (0), its commit (1) or clearing (2). |

Timers and animation frames run inside `vitrail_tick` (its clock); pointer calls deliver DOM events to listeners. A press
focuses the field, button, link or `[tabindex]` element under it; in a text field it places the caret (a double press
selects a word, a triple the line) and a drag selects.

## Input

| Function | Does |
|---|---|
| `vitrail_pointer_move(ui, x, y)` | The pointer in device px; updates `:hover`. Returns 1 when the hover changed. |
| `vitrail_pointer_down(ui, x, y)` / `int vitrail_pointer_up(ui, x, y, action, id, cap)` | A click; `up` returns 1 and copies the clicked element's `data-action` and `id` when it had one. |
| `int vitrail_hovered(ui, action, id, cap)` | The hovered element's `data-action` and `id`. |
| `int vitrail_hit(ui, x, y)` | 1 when the page takes the pointer there (an element that isn't `pointer-events: none`). |
| `int vitrail_scroll(ui, x, y, dx, dy)` | The wheel over (x, y): the scrollable element there scrolls by (dx, dy) device px (positive dy down), then its ancestors once it can't. 0 nothing moved; 1 the page must paint; 2 (GPU mode) the content the frame holds covers it: read `vitrail_scroll_states` and redraw, no paint. |
| `int vitrail_scroll_extents(ui, float out[], max)` | For overlay scrollbars: each GPU-scrolled container's own offset, visible size and content size (6 floats, device px). |
| `int vitrail_navigate(ui, dir)` | A gamepad step (0 up, 1 right, 2 down, 3 left): the focus moves to the nearest focusable element that way (the one in line first), its ring shows (`:focus-visible`) and it scrolls into view; with a select's list open, up and down move its highlight. With nothing focused, the top-left element takes the focus. 1 when something moved. |
| `int vitrail_activate(ui, action, id, cap)` | A gamepad's A: presses the focused element as the mouse would (opens a select, ticks a checkbox, clicks); 1 and its `data-action`/`id` for a click, 2 for anything else, 0 with nothing focused. With a select's list open, chooses the highlighted row. |
| `int vitrail_cancel(ui)` | A gamepad's B: closes a select's open list, else takes the focus away; 1 when it did either. |
| `int vitrail_value(ui, selector, buf, cap)` | The first match's value: a select's chosen option, a text field's text, `on`/empty for a checkbox or radio, else its `value` attribute. 1, 0 no match, or minus the bytes needed. |
| `int vitrail_select_value(ui, selector, value)` | Chooses the option with that value in the matching select (no events). |
| `int vitrail_cursor(ui)` | The cursor CSS asks for under the pointer: 0 default, 1 pointer (a hand), 2 text, 3 another. |
| `int vitrail_rect(ui, selector, float out[4])` | Where the first match is: x, y, width, height in device px. |

## Painting on the CPU (the fallback)

| Function | Does |
|---|---|
| `int vitrail_needs_paint(ui)` | 1 when something changed since the last paint. |
| `int vitrail_paint_start(ui)` | Styles, layout and the draw list on this thread, pixels (or the GPU frame) on the worker. 0 when nothing to do or a paint is running. |
| `int vitrail_paint_poll(ui, out, out_len, rect[4], mask, mask_len, params, max_regions, *regions)` | Takes a finished paint. Flags: 1 done, 2 `out` got new pixels inside `rect`, 4 the glass mask changed, 8 a GPU frame is ready. |
| `int vitrail_render(ui, out, len)` | Paints synchronously into `out` (RGBA8, premultiplied, rows top down). For tests and captures. |
| `u32 vitrail_backdrop(ui, mask, len, params, max_regions)` | The CPU path's glass regions (mask + per-region blur and colour matrix). |

## Painting on the GPU

`vitrail_set_gpu(ui, 2)` switches the page to GPU frames with soft edges (1: frames for multisampled targets, 0: CPU
pixels). After `vitrail_paint_poll` reports 8 (or after `vitrail_gpu_build` for a synchronous build), read the frame:

| Function | Gives |
|---|---|
| `vitrail_gpu_counts(ui, u32 counts[7])` | vertices, indices, batches, ramp rows, images, ops, whether the glyph atlas changed |
| `vitrail_gpu_copy(ui, vertices, vlen, indices, ilen, batches, blen)` | the buffers (a null `vertices` skips them): `Vertex` 60 bytes (x, y f32; colour RGBA8 premultiplied sRGB; u, v f32; p f32×4; meta u32 = material | clip1 << 8 | scroll container << 24; meta2 u32 = clip2 | clip3 << 16; q f32×4 image atlas rectangle), indices u32, `Batch` 20 bytes (kind, first, count, texture, shader clip) |
| `vitrail_gpu_ops(ui, out, len)` | `Op` 28 bytes (kind, batch, alpha, compose, mix, filter, filter_count); kinds in [Architecture](architecture.md#the-gpu-frame) |
| `vitrail_gpu_ramps(ui, out, len)` | gradient ramps, 256 RGBA8 per row |
| `vitrail_gpu_atlas(ui, out, len)` | the 2048×2048 R8 glyph atlas, when it changed (returns 1 then) |
| `vitrail_gpu_image(ui, index, u32 info[5], out, len)` | image `index`: info = id (2×u32), width, height, premultiplied; pixels RGBA8 when `out` is given. Upload each id once. |
| `u32 vitrail_gpu_image_texture(ui, index)` | the game texture image `index` stands for (0: an ordinary image) |
| `u32 vitrail_texture_id(name)` | the id of game texture `name` |
| `int vitrail_gpu_filters(ui, out, len)` | filter steps, 128 bytes each (kind 1 blur, 2 shadow, 3 colour matrix; σ, offset, colour, 4×5 matrix, bounds); returns the count |
| `int vitrail_gpu_clips(ui, float* out, len)` | shader clips, 16 floats each (x0 y0 x1 y1, x0 > x1 for "everything but"; rx ry per corner from top-left; the scroll container it moves with; three unused), referenced by vertices; returns the count |
| `int vitrail_gpu_image_atlas(ui, out, len)` | the 2048² RGBA8 image atlas when it changed (returns 1 then) |
| `int vitrail_gpu_layers(ui, out, len)` | composited layers, 80 bytes each (node id u64, inline, world and own transform 2×f32[6], bounds f32[4]); returns the count |
| `int vitrail_layer_states(ui, float* out, len)` | per layer 7 floats: the matrix from as painted to now (a b c d e f) and the opacity |
| `int vitrail_gpu_scrolls(ui, out, len)` | GPU-scrolled containers, 40 bytes each (node u64, enclosing index, offset as painted f32×2, clip box f32×4); returns the count |
| `int vitrail_scroll_states(ui, float* out, max)` | each GPU-scrolled container's offset now (2 floats, device px, enclosing ones included) |
| `int vitrail_gpu_dirty(ui, float* out, max)` | the page rectangles the frame changed (x0 y0 x1 y1, up to four): redraw kept textures only there; returns how many |
| `int vitrail_gpu_vertex_spans(ui, u32* out, max)` | the vertex spans (start, end) the last frame wrote, at most 32: the only vertices that changed |
| `int vitrail_gpu_vertex_range(ui, out, len, start, end)` | copies vertices start..end into the same place of `out`, a mirror of the whole vertex buffer |
| `int vitrail_gpu_changed(ui, u32 out[4])` | what the frame changed against the one before: vertices first..end, indices first..end (upload only those) |
| `int vitrail_gpu_build(ui, counts)` | builds the frame synchronously on this thread (tests, captures) |
