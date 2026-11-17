//! The page as triangles for the game's GPU (step 1 of the GPU painter): Blitz's draw commands, already in device
//! pixels, become one vertex buffer, one index buffer and a list of batches, each drawn with one material. Unity draws
//! them into a multisampled texture in sRGB space (so blending works as in a browser). Fills and strokes of any shape
//! are tessellated here; brushes, text, shadows and clips come in the next steps.

use anyrender::recording::RenderCommand;
use anyrender::{Paint, Scene};
use lyon_tessellation::geom::point;
use lyon_tessellation::path::Path;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, LineCap, LineJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use peniko::kurbo::{Affine, BezPath, PathEl};

/// One vertex (60 bytes): device px position, premultiplied sRGB colour (RGBA8), a texture coordinate, four parameters
/// a material may read (gradients, shadows), its material and clip, and its place in the image atlas.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
    pub color: u32,
    pub u: f32,
    pub v: f32,
    pub p: [f32; 4],
    /// The material (bits 0–7: SOLID, LINEAR, …), the first shader clip (bits 8–23: index + 1 into `Frame::clips`, 0
    /// none) and the scroll container the vertex moves with (bits 24–31: index + 1 into `Frame::scrolls`, 0 the page):
    /// one "uber" pass draws runs of any materials and clips in one call.
    pub meta: u32,
    /// The second and third shader clips (16 bits each).
    pub meta2: u32,
    /// An image's place in the image atlas (u0 v0 u1 v1, its exact texel rectangle), zero when not in it.
    pub q: [f32; 4],
}

/// A run of indices drawn with one material and state.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Batch {
    /// 0 solid colour (vertex colour).
    pub kind: u32,
    pub first: u32,
    pub count: u32,
    /// The texture it samples (images, the glyph atlas), 0 when none.
    pub texture: u32,
    /// The rounded-rectangle clip its shader applies per pixel (`Frame::clips` index + 1), 0 when none. (Stencil clips
    /// are ops; the depth they test is the painter's.)
    pub stencil: u32,
}

#[derive(Default)]
pub struct Frame {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
    /// Gradient ramps, one row of 256 premultiplied sRGB colours (RGBA8) per gradient brush.
    pub ramps: Vec<[u32; RAMP]>,
    /// The images the frame draws (batch.texture = index + 1).
    pub images: Vec<peniko::ImageData>,
    /// What to do, in order: draw a batch, or push/pop a clip or a group (see `Op`).
    pub ops: Vec<Op>,
    /// Filter steps of groups (see `Op::filter`).
    pub filters: Vec<FilterStep>,
    /// Elements composited on their own (an animated transform or opacity), in the order they start.
    pub layers: Vec<CompLayer>,
    /// Scroll containers whose content is held unscrolled (the GPU moves it).
    pub scrolls: Vec<ScrollInfo>,
    /// The scroll container vertices pushed now move with (index + 1, 0 the page).
    scroll: u32,
    /// Rounded-rectangle clips the shaders apply per pixel (referenced from `Vertex::meta`): x0 y0 x1 y1 (x0 > x1:
    /// everything but it), the corner radii rx ry for top-left, top-right, bottom-right, bottom-left (device px), then
    /// the scroll container it moves with (index + 1 into `scrolls`, 0: the page) and three unused floats.
    pub clips: Vec<[f32; 16]>,
    /// The shader clips open now, in three slots (index + 1 into `clips`, 0 for a free slot): every vertex pushed now
    /// carries them (`Vertex::meta`), so nested clips need neither the stencil nor a texture until the slots run out.
    slots: [u32; 3],
    /// What differs from the frame before: vertices [0]..[1] and indices [2]..[3] (up to the end when the counts
    /// changed), so the game uploads only that.
    pub changed: [u32; 4],
    /// The vertex spans this frame wrote (at most 32): the game uploads only these.
    pub vertex_spans: Vec<(u32, u32)>,
    /// The page rectangles (x0 y0 x1 y1, device px, at most four) that differ from the frame before: the game may
    /// redraw only those. One rectangle over everything when the frame's structure (groups, clips, layers) changed.
    pub dirty: Vec<[f32; 4]>,
    /// The vertex pool and the clip table while the frame is built.
    pool: VertexPool,
    clip_table: StableTable<[f32; 16]>,
    /// Groups being recorded (innermost last): every draw pushed now is logged into each.
    recording: Vec<GroupRecording>,
}

/// One step of the frame. kind: 0 draw `batch`; 1 push a clip (stencil) of shape `batch`; 2 pop it; 3 push a group (an
/// offscreen layer) clipped by `batch`; 4 pop it, compositing with `alpha`, `compose` (peniko Compose) and `mix`;
/// 5 backdrop-filter: what lies under shape `batch` (the scene and the page so far) run through the filter steps and put
/// back there, before the element itself draws; 6 and 7: the start and end of composited layer `batch` (see `CompLayer`):
/// what lies between is drawn once into the layer's own texture and laid on the page with the layer's current matrix
/// and opacity every frame.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Op {
    pub kind: u32,
    pub batch: u32,
    pub alpha: f32,
    pub compose: u32,
    pub mix: u32,
    /// For a group's pop: its filter steps, `filters[filter .. filter + filter_count]`, run before compositing.
    pub filter: u32,
    pub filter_count: u32,
}

/// One step of a CSS `filter` on a group, in device px: 1 Gaussian blur (`sigma`), 2 drop shadow (`sigma`, `dx`, `dy`,
/// premultiplied sRGB `color`), 3 colour matrix (`m`, the SVG feColorMatrix on unpremultiplied sRGB, 4×5 row-major).
/// `bounds` (x0, y0, x1, y1) is where the group can have pixels: the filter passes run only there.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FilterStep {
    pub kind: u32,
    pub sigma: f32,
    pub dx: f32,
    pub dy: f32,
    pub color: [f32; 4],
    pub m: [f32; 20],
    pub bounds: [f32; 4],
}

/// A scroll container whose content the frame holds unscrolled: its node, the enclosing one (index + 1, 0 none), its
/// scroll offset when painted and its clip box (device px).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ScrollInfo {
    pub id: u64,
    pub parent: u32,
    pub _pad: u32,
    pub offset: [f32; 2],
    pub clip: [f32; 4],
}

/// An element composited on its own: its node, whether it could not be (`inline`: it sits inside a group, so it was drawn
/// with the page and moving it needs a repaint), its world transform as painted, its own CSS transform then, and where
/// its pixels are (device px, x0 y0 x1 y1).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct CompLayer {
    pub id: u64,
    pub inline: u32,
    pub _pad: u32,
    pub world: [f32; 6],
    pub local: [f32; 6],
    pub bounds: [f32; 4],
}

fn coeffs(t: Affine) -> [f32; 6] { let c = t.as_coeffs(); [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32, c[5] as f32] }

/// The part of `new` that differs from `old` (compared as plain bytes per element): from the first element that differs
/// to the end of the last difference, or to the end of `new` when the lengths differ (what follows a change then moved).
pub fn changed_range<T: Copy>(old: &[T], new: &[T]) -> (u32, u32) {
    let size = std::mem::size_of::<T>();
    let bytes = |v: &[T]| unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * size) };
    let (a, b) = (bytes(old), bytes(new));
    let first = a.chunks_exact(size).zip(b.chunks_exact(size)).position(|(x, y)| x != y).unwrap_or(old.len().min(new.len()));
    if old.len() != new.len() { return (first as u32, new.len() as u32); }
    if first == new.len() { return (0, 0); }
    let last = a.chunks_exact(size).zip(b.chunks_exact(size)).rposition(|(x, y)| x != y).unwrap_or(first);
    (first as u32, last as u32 + 1)
}

/// A layer's filter as steps for the GPU; effects it cannot run yet are skipped.
fn filter_steps(f: &anyrender::filters::Filter, t: Affine, bounds: [f32; 4], out: &mut Vec<FilterStep>) {
    use anyrender::filters::FilterEffect;
    let c = t.as_coeffs();
    let scale = (c[0] * c[3] - c[1] * c[2]).abs().sqrt() as f32;
    for node in f.nodes() {
        let mut st = FilterStep { bounds, ..Default::default() };
        match &node.effect {
            FilterEffect::GaussianBlur(b) => { if b.std_deviation <= 0.0 { continue; } st.kind = 1; st.sigma = b.std_deviation * scale; }
            FilterEffect::DropShadow(d) => {
                st.kind = 2;
                st.sigma = d.std_deviation * scale;
                let o = (t * peniko::kurbo::Point::new(d.dx as f64, d.dy as f64)) - (t * peniko::kurbo::Point::ORIGIN);
                (st.dx, st.dy) = (o.x as f32, o.y as f32);
                let [r, g, b, a] = d.color.components;
                st.color = [r * a, g * a, b * a, a];
            }
            FilterEffect::ColorMatrix(m) => { st.kind = 3; st.m = m.0; }
            _ => continue,
        }
        out.push(st);
    }
}

/// A batch that is only a clip shape (drawn into the stencil or as a group's footprint).
pub const CLIP: u32 = 100;

pub const RAMP: usize = 256;

/// Material numbers (the shader pass a batch uses).
pub const SOLID: u32 = 0;
pub const LINEAR: u32 = 1;
pub const RADIAL: u32 = 2;
pub const SWEEP: u32 = 3;
pub const IMAGE: u32 = 4;
pub const GLYPH: u32 = 8;
pub const BOX_SHADOW: u32 = 9;
/// An inset box-shadow: the colour where the blurred hole doesn't cover (pass 11; 10 is the blend-mode pass).
pub const INSET_SHADOW: u32 = 11;
/// The uber material: each vertex says which material it is (`Vertex::meta`); a batch of it mixes materials.
pub const UBER: u32 = 12;
/// The image atlas: small images that clamp at their edges, packed RGBA8 into IMAGE_ATLAS², so drawing them doesn't break
/// a batch.
pub const IMAGE_ATLAS: u32 = 2048;
const IMAGE_ATLAS_MAX: u32 = 512;
/// Shader clips a frame can hold (the clip table is a texture this tall).
pub const MAX_CLIPS: usize = 16383;
/// Scroll containers a frame can scroll on the GPU (the rest scroll by painting).
pub const MAX_SCROLLS: usize = 255;

/// The glyph atlas: one coverage byte per pixel, ATLAS×ATLAS.
pub const ATLAS: u32 = 2048;
/// The biggest glyph bitmap drawn through the atlas (bigger glyphs are rare: headings at huge sizes).
const GLYPH_MAX: u32 = 256;
/// Text bigger than this (device px) is drawn from outlines rather than the atlas.
const GLYPH_OUTLINE: f32 = 160.0;

/// Glyphs rasterized once by the same CPU painter as the rest of Blitz (so text looks exactly as it does there),
/// packed in shelves into the atlas, and found again by font, glyph, size and quarter-pixel offset.
pub struct GlyphCache {
    pub atlas: Vec<u8>,
    /// Set when the atlas got new glyphs since the game last copied it.
    pub dirty: bool,
    map: std::collections::HashMap<GlyphKey, Option<GlyphEntry>>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    painter: Option<anyrender_vello_cpu::VelloCpuImageRenderer>,
    buf: Vec<u8>,
    /// Set when the atlas filled up and started over while building a frame (the frame must be built again).
    pub overflowed: bool,
    /// Goes up when the atlas is cleared: glyph quads made before point at nothing (reused groups check it).
    pub epoch: u64,
    /// Shapes already turned into triangles, kept between frames (they travel with the glyphs to the worker).
    pub tess: TessCache,
    /// Colour glyphs (emoji) as images, by font, glyph and strike size.
    color: std::collections::HashMap<(u64, u32, u32, u32), Option<(peniko::ImageData, Affine)>>,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct GlyphKey { font: u64, index: u32, glyph: u32, size: u32, sub: u8, embolden: (i32, i32), skew: i32 }

#[derive(Clone, Copy)]
struct GlyphEntry { x: u32, y: u32, w: u32, h: u32, left: f32, top: f32 }

/// Small images packed into one texture (shelves, a one-pixel border repeating the edge so bilinear reads stay inside).
pub struct ImageAtlas {
    pub pixels: Vec<u8>,
    pub dirty: bool,
    map: std::collections::HashMap<(u64, u32, u32), Option<[f32; 4]>>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
}

impl Default for ImageAtlas {
    fn default() -> Self { Self { pixels: Vec::new(), dirty: false, map: Default::default(), shelf_x: 0, shelf_y: 0, shelf_h: 0 } }
}

impl ImageAtlas {
    /// The image's rectangle in the atlas (normalized), placing it when new; None when it is too
    /// big or the atlas is full (it is then drawn from its own texture).
    pub fn place(&mut self, data: &peniko::ImageData) -> Option<[f32; 4]> {
        let key = (data.data.id(), data.width, data.height);
        if let Some(r) = self.map.get(&key) { return *r; }
        let (w, h) = (data.width, data.height);
        let fits = w > 0 && h > 0 && w <= IMAGE_ATLAS_MAX && h <= IMAGE_ATLAS_MAX && data.data.data().len() >= (w * h * 4) as usize
            && matches!(data.format, peniko::ImageFormat::Rgba8);
        let r = if !fits { None } else {
            let (bw, bh) = (w + 2, h + 2);
            if self.shelf_x + bw > IMAGE_ATLAS { self.shelf_x = 0; self.shelf_y += self.shelf_h; self.shelf_h = 0; }
            if self.shelf_y + bh > IMAGE_ATLAS { None } else {
                if self.pixels.is_empty() { self.pixels = vec![0; (IMAGE_ATLAS * IMAGE_ATLAS * 4) as usize]; }
                let (ax, ay) = (self.shelf_x, self.shelf_y);
                self.shelf_x += bw;
                self.shelf_h = self.shelf_h.max(bh);
                let src = data.data.data();
                for y in 0..bh {
                    let sy = (y as i64 - 1).clamp(0, h as i64 - 1) as u32;
                    for x in 0..bw {
                        let sx = (x as i64 - 1).clamp(0, w as i64 - 1) as u32;
                        let si = ((sy * w + sx) * 4) as usize;
                        let di = (((ay + y) * IMAGE_ATLAS + ax + x) * 4) as usize;
                        self.pixels[di..di + 4].copy_from_slice(&src[si..si + 4]);
                    }
                }
                self.dirty = true;
                let a = IMAGE_ATLAS as f32;
                Some([(ax + 1) as f32 / a, (ay + 1) as f32 / a, (ax + 1 + w) as f32 / a, (ay + 1 + h) as f32 / a])
            }
        };
        self.map.insert(key, r);
        r
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self { epoch: 0, atlas: vec![0; (ATLAS * ATLAS) as usize], dirty: true, map: Default::default(), shelf_x: 0, shelf_y: 0, shelf_h: 0, painter: None, buf: Vec::new(), overflowed: false, tess: TessCache::default(), color: Default::default() }
    }
}

impl GlyphCache {
    /// The glyph's place in the atlas, rasterizing it if new: Ok(None) when it has no ink (a space) or is too big for
    /// the atlas, Err(()) when the atlas is full.
    fn get(&mut self, run: &anyrender::recording::GlyphRunCommand, size: f32, glyph: u32, sub: u8) -> Result<Option<GlyphEntry>, ()> {
        // Emboldening is in the run's user units: at a bigger raster size it grows with it.
        let embolden = run.embolden * (size / run.font_size.max(1e-6)) as f64;
        use skrifa::MetadataProvider;
        let key = GlyphKey {
            font: run.font_data.data.id(), index: run.font_data.index, glyph,
            size: (size * 64.0).round() as u32, sub,
            embolden: ((embolden.x * 256.0).round() as i32, (embolden.y * 256.0).round() as i32),
            skew: run.glyph_transform.map(|t| (t.as_coeffs()[2] * 1024.0).round() as i32).unwrap_or(0),
        };
        if let Some(e) = self.map.get(&key) { return Ok(*e); }
        let Ok(font) = skrifa::FontRef::from_index(run.font_data.data.data(), run.font_data.index) else { return Ok(None) };
        let bounds = font.glyph_metrics(skrifa::instance::Size::new(size), skrifa::instance::LocationRef::default())
            .bounds(skrifa::GlyphId::new(glyph));
        // Font units are y up; the bitmap is y down with room for antialiasing, emboldening and a synthetic slant.
        let pad = 3.0 + (embolden.x.abs().max(embolden.y.abs()) as f32) * 2.0;
        // Fonts whose outlines skrifa can't measure (some system collections) get a generous em box instead; the empty
        // part of the bitmap costs only atlas space.
        let fs = size;
        let (x0, x1, y0, y1) = match bounds { Some(b) if b.x_max > b.x_min => (b.x_min, b.x_max, b.y_min, b.y_max), _ => (-0.25 * fs, 1.25 * fs, -0.4 * fs, 1.1 * fs) };
        let slant = run.glyph_transform.map(|t| t.as_coeffs()[2].abs() as f32).unwrap_or(0.0) * y1.abs().max(y0.abs());
        let left = (x0 - pad - slant).floor();
        let top = (-y1 - pad).floor();
        let w = ((x1 + pad + slant).ceil() - left).max(1.0) as u32 + 1;
        let h = ((-y0 + pad).ceil() - top).max(1.0) as u32;
        if w > GLYPH_MAX || h > GLYPH_MAX {
            self.map.insert(key, None);
            return Ok(None);
        }
        // A shelf for it (a full atlas starts over; the frame then repacks what it uses).
        if self.shelf_x + w > ATLAS { self.shelf_x = 0; self.shelf_y += self.shelf_h; self.shelf_h = 0; }
        if self.shelf_y + h > ATLAS { return Err(()); }
        let (ax, ay) = (self.shelf_x, self.shelf_y);
        self.shelf_x += w + 1;
        self.shelf_h = self.shelf_h.max(h + 1);
        // Rasterize: the glyph's origin at (-left + sub/4, -top) in a GLYPH_MAX square, white, by the CPU painter.
        use anyrender::{ImageRenderer, PaintScene};
        let painter = self.painter.get_or_insert_with(|| anyrender_vello_cpu::VelloCpuImageRenderer::new(GLYPH_MAX, GLYPH_MAX));
        // The painter draws over what is in the buffer: start each glyph on a clean one.
        self.buf.clear();
        self.buf.resize((GLYPH_MAX * GLYPH_MAX * 4) as usize, 0);
        painter.reset();
        let ox = -left as f64 + sub as f64 / 4.0;
        let oy = -top as f64;
        let coords: &[i16] = &run.normalized_coords;
        painter.render(|s| {
            s.draw_glyphs(&run.font_data, size, run.hint, coords, embolden, peniko::Fill::NonZero,
                &Paint::Solid(peniko::Color::WHITE), 1.0, Affine::translate((ox, oy)), run.glyph_transform,
                std::iter::once(anyrender::Glyph { id: glyph, x: 0.0, y: 0.0 }));
        }, &mut self.buf);
        for row in 0..h {
            for col in 0..w {
                self.atlas[((ay + row) * ATLAS + ax + col) as usize] = self.buf[((row * GLYPH_MAX + col) * 4 + 3) as usize];
            }
        }
        self.dirty = true;
        let e = GlyphEntry { x: ax, y: ay, w, h, left, top };
        self.map.insert(key, Some(e));
        Ok(Some(e))
    }

    /// A colour glyph's picture (sbix or CBDT bitmaps: Apple's and Noto's emoji) and where it goes relative to the
    /// glyph's origin, in the run's user units; None for ordinary glyphs. Placement as Skia's (and vello's) fontations
    /// port does it, with Apple's fixed shift for Apple Color Emoji.
    fn color_glyph(&mut self, run: &anyrender::recording::GlyphRunCommand, glyph: u32, device_ppem: f32) -> Option<(peniko::ImageData, Affine)> {
        use skrifa::MetadataProvider;
        let key = (run.font_data.data.id(), run.font_data.index, glyph, device_ppem.round().max(1.0) as u32);
        if let Some(hit) = self.color.get(&key) { return hit.clone(); }
        let made = (|| {
            let font = skrifa::FontRef::from_index(run.font_data.data.data(), run.font_data.index).ok()?;
            let strikes = skrifa::bitmap::BitmapStrikes::new(&font);
            let bitmap = strikes.glyph_for_size(skrifa::instance::Size::new(device_ppem.max(1.0)), skrifa::GlyphId::new(glyph))?;
            let pixels: Vec<u8> = match bitmap.data {
                skrifa::bitmap::BitmapData::Png(png) => image::load_from_memory(png).ok()?.to_rgba8().into_raw(),
                skrifa::bitmap::BitmapData::Bgra(bgra) => {
                    // Premultiplied BGRA: to straight RGBA.
                    bgra.chunks_exact(4).flat_map(|p| {
                        let a = p[3] as f32 / 255.0;
                        let un = |v: u8| if a > 0.0 { ((v as f32 / a).min(255.0)) as u8 } else { 0 };
                        [un(p[2]), un(p[1]), un(p[0]), p[3]]
                    }).collect()
                }
                _ => return None,
            };
            let (w, h) = (bitmap.width, bitmap.height);
            if pixels.len() < (w * h * 4) as usize { return None; }
            let upem = skrifa::raw::TableProvider::head(&font).ok()?.units_per_em() as f32;
            let fs = run.font_size;
            let apple = font.localized_strings(skrifa::string::StringId::FAMILY_NAME).english_or_first()
                .map(|n| n.to_string().contains("Apple Color Emoji")).unwrap_or(false);
            let outer = if apple { peniko::kurbo::Vec2::new(0.0, fs as f64 / 8.0) } else {
                peniko::kurbo::Vec2::new((bitmap.bearing_x * fs / upem) as f64, -(bitmap.bearing_y * fs / upem) as f64)
            };
            let mut t = Affine::translate(outer) * Affine::scale((fs / bitmap.ppem_y) as f64)
                * Affine::translate((bitmap.inner_bearing_x as f64, -bitmap.inner_bearing_y as f64));
            if matches!(bitmap.placement_origin, skrifa::bitmap::Origin::BottomLeft) { t = t * Affine::translate((0.0, -(h as f64))); }
            let data = peniko::ImageData { data: peniko::Blob::new(std::sync::Arc::new(pixels)), format: peniko::ImageFormat::Rgba8,
                alpha_type: peniko::ImageAlphaType::Alpha, width: w, height: h };
            Some((data, t))
        })();
        if self.color.len() > 2048 { self.color.clear(); }
        self.color.insert(key, made.clone());
        made
    }

    /// Forgets every glyph (when the atlas is full); the next frame rasterizes what it needs again.
    fn clear(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.map.clear();
        self.atlas.iter_mut().for_each(|b| *b = 0);
        self.shelf_x = 0; self.shelf_y = 0; self.shelf_h = 0;
        self.dirty = true;
    }
}

const TOLERANCE: f32 = 0.05;

fn premul(c: peniko::Color, alpha: f32) -> u32 {
    let [r, g, b, a] = c.components;
    let a = (a * alpha).clamp(0.0, 1.0);
    let q = |v: f32| ((v * a).clamp(0.0, 1.0) * 255.0).round() as u32;
    q(r) | (q(g) << 8) | (q(b) << 16) | (((a * 255.0).round() as u32) << 24)
}

/// How a brush paints a vertex: its material, texture, colour, and how its texture coordinate and parameters follow
/// from the vertex's device position.
struct BrushMap {
    kind: u32,
    texture: u32,
    color: u32,
    /// Device px → brush space (the inverse of the shape's and the brush's transforms).
    inv: Affine,
    map: fn(&BrushMap, peniko::kurbo::Point) -> ([f32; 2], [f32; 4]),
    p: [f32; 4],
    origin: peniko::kurbo::Point,
    dir: peniko::kurbo::Vec2,
    size: [f32; 2],
    /// The image's place in the atlas (zero when not in it).
    q: [f32; 4],
}

/// A gradient's colours sampled into 256 steps, interpolated in sRGB with premultiplied alpha (as CSS does).
fn ramp(g: &peniko::Gradient) -> [u32; RAMP] {
    let stops: Vec<(f32, [f32; 4])> = g.stops.iter().map(|s| {
        let c = s.color.to_alpha_color::<peniko::color::Srgb>().components;
        (s.offset, [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]])
    }).collect();
    let mut out = [0u32; RAMP];
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f32 / (RAMP - 1) as f32;
        let c = if stops.is_empty() { [0.0; 4] } else if t <= stops[0].0 { stops[0].1 } else if t >= stops[stops.len() - 1].0 { stops[stops.len() - 1].1 } else {
            let k = stops.windows(2).position(|w| t >= w[0].0 && t <= w[1].0).unwrap_or(0);
            let (a, b) = (stops[k], stops[k + 1]);
            let f = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
            [a.1[0] + (b.1[0] - a.1[0]) * f, a.1[1] + (b.1[1] - a.1[1]) * f, a.1[2] + (b.1[2] - a.1[2]) * f, a.1[3] + (b.1[3] - a.1[3]) * f]
        };
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        *o = q(c[0]) | (q(c[1]) << 8) | (q(c[2]) << 16) | (q(c[3]) << 24);
    }
    out
}

impl Frame {
    /// How `brush`, under `transform` and its own `brush_transform`, paints.
    fn brush(&mut self, tess: &mut TessCache, brush: &Paint, transform: Affine, brush_transform: Option<Affine>) -> BrushMap {
        let inv = (transform * brush_transform.unwrap_or(Affine::IDENTITY)).inverse();
        let none = |_: &BrushMap, _: peniko::kurbo::Point| ([0.0f32; 2], [0.0f32; 4]);
        let mut m = BrushMap { kind: SOLID, texture: 0, color: 0xffffffff, inv, map: none, p: [0.0; 4], origin: Default::default(), dir: Default::default(), size: [1.0, 1.0], q: [0.0; 4] };
        match brush {
            Paint::Solid(c) => m.color = premul(*c, 1.0),
            Paint::Gradient(g) => {
                // One row per distinct set of stops in a frame; the colours are worked out once and kept.
                let mut k = Key::new();
                k.word(g.interpolation_alpha_space as u64);
                for st in g.stops.iter() { let c = st.color.to_alpha_color::<peniko::color::Srgb>().components; k.f(st.offset as f64); for v in c { k.f(v as f64); } }
                let key = k.0;
                let row = tess.ramp_table.get(key, usize::MAX, || ramp(g)).unwrap_or(0) as f32;
                for r in &mut self.recording { r.ramps.push(key); }
                m.color = 0xffffffff;
                let extend = g.extend as u32 as f32;
                match g.kind {
                    peniko::GradientKind::Linear(l) => {
                        m.kind = LINEAR;
                        m.origin = l.start;
                        m.dir = l.end - l.start;
                        m.p = [row, extend, 0.0, 0.0];
                        m.map = |m, pt| {
                            let g = m.inv * pt;
                            let d2 = m.dir.hypot2().max(1e-12);
                            ([((g - m.origin).dot(m.dir) / d2) as f32, 0.0], m.p)
                        };
                    }
                    peniko::GradientKind::Radial(r) => {
                        // Concentric circles (what CSS radial gradients give); the end circle's centre.
                        m.kind = RADIAL;
                        m.origin = r.end_center;
                        m.p = [row, extend, r.start_radius, r.end_radius];
                        m.map = |m, pt| { let g = m.inv * pt - m.origin; ([g.x as f32, g.y as f32], m.p) };
                    }
                    peniko::GradientKind::Sweep(s) => {
                        m.kind = SWEEP;
                        m.origin = s.center;
                        m.p = [row, extend, s.start_angle, s.end_angle];
                        m.map = |m, pt| { let g = m.inv * pt - m.origin; ([g.x as f32, g.y as f32], m.p) };
                    }
                }
            }
            Paint::Image(img) => {
                let data = &img.image;
                m.kind = IMAGE;
                // A small image that clamps at its edges goes into the atlas (no texture of its own, no batch break).
                let pad = matches!(img.sampler.x_extend, peniko::Extend::Pad) && matches!(img.sampler.y_extend, peniko::Extend::Pad);
                match if pad { tess.image_atlas.place(data) } else { None } {
                    Some(q) => m.q = q,
                    None => {
                        let index = match self.images.iter().position(|i| i.data.id() == data.data.id() && i.width == data.width && i.height == data.height) {
                            Some(i) => i,
                            None => { self.images.push(data.clone()); self.images.len() - 1 }
                        };
                        m.texture = index as u32 + 1;
                    }
                }
                m.color = 0xffffffff;
                m.size = [data.width as f32, data.height as f32];
                let premultiplied = matches!(data.alpha_type, peniko::ImageAlphaType::AlphaPremultiplied);
                m.p = [img.sampler.alpha, premultiplied as u32 as f32, img.sampler.x_extend as u32 as f32, img.sampler.y_extend as u32 as f32];
                m.map = |m, pt| { let g = m.inv * pt; ([g.x as f32 / m.size[0], g.y as f32 / m.size[1]], m.p) };
            }
            _ => m.color = premul(peniko::Color::from_rgba8(255, 0, 255, 255), 1.0),
        }
        m
    }
}

impl BrushMap {
    fn vertex(&self, x: f32, y: f32) -> Vertex {
        let (uv, p) = (self.map)(self, peniko::kurbo::Point::new(x as f64, y as f64));
        Vertex { x, y, color: self.color, u: uv[0], v: uv[1], p, meta: 0, meta2: 0, q: self.q }
    }
}

/// A kurbo path, moved by `t`, as a lyon path.
fn to_lyon(path: &BezPath, t: Affine) -> Path {
    let mut b = Path::builder().with_svg();
    let p = |pt: peniko::kurbo::Point| { let q = t * pt; point(q.x as f32, q.y as f32) };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(a) => { b.move_to(p(a)); }
            PathEl::LineTo(a) => { b.line_to(p(a)); }
            PathEl::QuadTo(c, a) => { b.quadratic_bezier_to(p(c), p(a)); }
            PathEl::CurveTo(c1, c2, a) => { b.cubic_bezier_to(p(c1), p(c2), p(a)); }
            PathEl::ClosePath => { b.close(); }
        }
    }
    b.build()
}


/// Coverage `cov` (0..1) folded into a vertex colour: a premultiplied colour scales all four channels.
fn scale_color(c: u32, cov: f32) -> u32 {
    let q = |sh: u32| ((((c >> sh) & 255) as f32 * cov).round() as u32) << sh;
    q(0) | q(8) | q(16) | q(24)
}

/// A shape's triangles in device px without its brush: x, y and edge coverage per vertex.
type Geo = VertexBuffers<[f32; 3], u32>;

/// Shapes turned into triangles, found again by their outline, transform and style, so a frame that differs from the
/// last in one button tessellates only that button. Entries unused for a while are dropped.
#[derive(Default)]
pub struct TessCache {
    map: std::collections::HashMap<u64, (Geo, u32)>,
    /// Gradient colours by their stops.
    ramps: std::collections::HashMap<u64, [u32; RAMP]>,
    /// Small images, packed so they draw in the same batch as everything else.
    pub image_atlas: ImageAtlas,
    /// Stable vertex places across frames.
    pub pool: VertexPool,
    /// Clip shapes recognized as (inverted) rounded rectangles, by outline and transform.
    clip_shapes: std::collections::HashMap<u64, (Option<[f32; 12]>, u32)>,
    /// The last frame's structure (ops without their batch numbers), to tell when only draws changed.
    structure: u64,
    /// Gradient ramps and shader clips with stable rows.
    ramp_table: StableTable<[u32; RAMP]>,
    clip_table: StableTable<[f32; 16]>,
    generation: u32,
    /// How many shapes the last frame tessellated anew (for measuring).
    pub misses: u32,
    /// What each shared recorded piece (an element Blitz painted earlier) drew last time, by its address: an unchanged
    /// piece appends those draws again instead of being walked (see `GroupDraws`).
    groups: std::collections::HashMap<usize, GroupDraws>,
}

/// One draw a group made: its texture, its vertices' place and pool key, its indices (relative to the place).
#[derive(Clone)]
struct GroupDraw { texture: u32, at: u32, n: u32, key: u64, indices: Vec<u32> }

/// What a group drew, and what that holds for: the shader clips and scroll space open around it, the glyph atlas.
struct GroupDraws {
    /// The piece itself, kept so its address stays its own while this is kept.
    _piece: std::sync::Arc<Vec<RenderCommand>>,
    context: (u32, u32, u64),
    draws: Vec<GroupDraw>,
    ramps: Vec<u64>,
    /// Shader clip rows its draws name (kept alive on a replay, as the gradient rows).
    clips: Vec<u64>,
    used: u32,
}

/// A group being recorded while it is walked: what its draws were (None once it did something a group can't replay:
/// a layer, a marker, an image of its own).
struct GroupRecording { end: usize, ptr: usize, piece: std::sync::Arc<Vec<RenderCommand>>, context: (u32, u32, u64), draws: Option<Vec<GroupDraw>>, ramps: Vec<u64>, clips: Vec<u64> }

/// Stable places for vertices across frames: each draw's vertices are found by their content, so a draw that didn't
/// change keeps its place and nothing of it is uploaded again; a changed one gets a new place (a freed block or the
/// end) and its old place is freed. Draw order lives in the index buffer, which is rebuilt each frame.
#[derive(Default)]
pub struct VertexPool {
    slots: std::collections::HashMap<u64, (u32, u32, u32)>,
    /// Freed blocks by their length (a draw takes a block of exactly its size; others go at the end).
    free: std::collections::HashMap<u32, Vec<u32>>,
    freed_vertices: usize,
    generation: u32,
    /// The length of the vertex buffer this pool describes (the next frame must start from that buffer).
    len: usize,
    live: usize,
    /// The spans written this frame (start, end), sorted and merged at the end of the frame.
    pub written: Vec<(u32, u32)>,
    /// Where each draw freed this frame was, and each draw written (x0 y0 x1 y1): what must be redrawn.
    pub freed: Vec<[f32; 4]>,
    pub placed: Vec<[f32; 4]>,
}

/// Vertices as their bytes (for exact comparison).
fn bytes_of(v: &[Vertex]) -> &[u8] { unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) } }

/// A quick hash of vertices as 32-bit words.
fn hash_vertices(v: &[Vertex]) -> u64 {
    let words = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u32, v.len() * std::mem::size_of::<Vertex>() / 4) };
    let mut h: u64 = 0x9E3779B97F4A7C15 ^ v.len() as u64;
    for &w in words { h = (h ^ w as u64).wrapping_mul(0x100000001B3).rotate_left(29); }
    h
}

impl VertexPool {
    /// Starts a frame on `vertices` (the last frame's buffer); a buffer that doesn't match starts the pool afresh.
    fn begin(&mut self, vertices: &mut Vec<Vertex>) {
        self.generation = self.generation.wrapping_add(1);
        self.written.clear();
        self.placed.clear();
        // Too fragmented (or not the buffer it describes): start again, everything is written anew.
        // Too much of it freed (blocks only fit draws of their exact size): start again.
        if vertices.len() != self.len || (self.len > 65536 && self.freed_vertices * 2 > self.len) {
            self.slots.clear();
            self.free.clear();
            self.freed_vertices = 0;
            vertices.clear();
            self.len = 0;
        }
    }

    /// The place of these vertices in `vertices`: where they already are, or a new place they are written to.
    fn place(&mut self, vertices: &mut Vec<Vertex>, new: &[Vertex]) -> u32 { self.place_keyed(vertices, new).0 }

    /// Keeps a place a reused group drew last frame (found by its key): false when it is gone (freed or moved).
    fn keep(&mut self, key: u64, at: u32, n: u32) -> bool {
        match self.slots.get_mut(&key) {
            Some(e) if e.0 == at && e.1 == n => { e.2 = self.generation; true }
            _ => false,
        }
    }

    /// `place`, also giving the key the place is kept under.
    fn place_keyed(&mut self, vertices: &mut Vec<Vertex>, new: &[Vertex]) -> (u32, u64) {
        let n = new.len() as u32;
        // A cheap key (the count and three vertices), confirmed by comparing the bytes: far quicker than hashing them
        // all, and exact. A clash with different content falls back to the full hash as the key.
        let mut key = hash_vertices(&[new[0], new[new.len() / 2], new[new.len() - 1]]) ^ (n as u64).wrapping_mul(0x9E3779B97F4A7C15);
        if let Some(e) = self.slots.get(&key) {
            let same = e.1 == n && bytes_of(&vertices[e.0 as usize..(e.0 + n) as usize]) == bytes_of(new);
            if same { let e = self.slots.get_mut(&key).unwrap(); e.2 = self.generation; return (e.0, key); }
            key = hash_vertices(new);
            if let Some(e) = self.slots.get(&key) {
                if e.1 == n && bytes_of(&vertices[e.0 as usize..(e.0 + n) as usize]) == bytes_of(new) {
                    let e = self.slots.get_mut(&key).unwrap(); e.2 = self.generation; return (e.0, key);
                }
            }
        }
        let at = match self.free.get_mut(&n).and_then(|v| v.pop()) {
            Some(off) => {
                self.freed_vertices -= n as usize;
                vertices[off as usize..(off + n) as usize].copy_from_slice(new);
                off
            }
            None => { let off = vertices.len() as u32; vertices.extend_from_slice(new); off }
        };
        match self.written.last_mut() { Some(w) if w.1 == at => w.1 = at + n, _ => self.written.push((at, at + n)) }
        self.placed.push(bounds_of(new));
        self.slots.insert(key, (at, n, self.generation));
        (at, key)
    }

    /// Ends a frame: places no draw used are freed (neighbouring free blocks merged).
    fn end(&mut self, vertices: &[Vertex]) {
        let g = self.generation;
        let free = &mut self.free;
        let freed = &mut self.freed;
        let mut freed_vertices = 0;
        freed.clear();
        self.slots.retain(|_, e| {
            let keep = e.2 == g;
            if !keep {
                free.entry(e.1).or_default().push(e.0);
                freed_vertices += e.1 as usize;
                freed.push(bounds_of(&vertices[e.0 as usize..(e.0 + e.1) as usize]));
            }
            keep
        });
        self.freed_vertices += freed_vertices;
        self.len = vertices.len();
        self.live = self.slots.values().map(|e| e.1 as usize).sum();
        // The written spans, sorted, merged when they touch, and at most 32 (the closest ones joined).
        self.written.sort_unstable();
        let mut spans: Vec<(u32, u32)> = Vec::with_capacity(self.written.len());
        for &(a, b) in &self.written {
            match spans.last_mut() { Some(m) if m.1 >= a => m.1 = m.1.max(b), _ => spans.push((a, b)) }
        }
        while spans.len() > 32 {
            let i = (0..spans.len() - 1).min_by_key(|&i| spans[i + 1].0 - spans[i].1).unwrap();
            spans[i].1 = spans[i + 1].1;
            spans.remove(i + 1);
        }
        self.written = spans;
    }
}

/// Rows that keep their index across frames, found by a key (gradient ramps by their stops, clips by their shape): a
/// vertex that names a row stays the same while the row is used, so its draw keeps its place in the vertex pool.
pub struct StableTable<T> {
    map: std::collections::HashMap<u64, (u32, u32)>,
    pub rows: Vec<T>,
    free: Vec<u32>,
    generation: u32,
}

impl<T> Default for StableTable<T> {
    fn default() -> Self { Self { map: Default::default(), rows: Vec::new(), free: Vec::new(), generation: 0 } }
}

impl<T: Copy> StableTable<T> {
    fn begin(&mut self) { self.generation = self.generation.wrapping_add(1); }
    /// The row for `key`, made by `make` when new (in a freed row or at the end); None when `limit` rows are taken.
    fn get(&mut self, key: u64, limit: usize, make: impl FnOnce() -> T) -> Option<u32> {
        if let Some(e) = self.map.get_mut(&key) { e.1 = self.generation; return Some(e.0); }
        let i = match self.free.pop() {
            Some(i) => { self.rows[i as usize] = make(); i }
            None => { if self.rows.len() >= limit { return None; } self.rows.push(make()); (self.rows.len() - 1) as u32 }
        };
        self.map.insert(key, (i, self.generation));
        Some(i)
    }
    /// Marks `key`'s row used this frame (a reused group's gradient); false when it is gone.
    fn touch(&mut self, key: u64) -> bool {
        match self.map.get_mut(&key) { Some(e) => { e.1 = self.generation; true } None => false }
    }
    /// Frees the rows no draw used this frame.
    fn end(&mut self) {
        let g = self.generation;
        let free = &mut self.free;
        self.map.retain(|_, e| { let keep = e.1 == g; if !keep { free.push(e.0); } keep });
    }
}

/// The box (x0 y0 x1 y1) around some vertices.
fn bounds_of(v: &[Vertex]) -> [f32; 4] {
    v.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, v| [b[0].min(v.x), b[1].min(v.y), b[2].max(v.x), b[3].max(v.y)])
}

/// Boxes grown by a pixel (antialiasing), overlapping ones joined, then the pair whose union adds least area joined
/// until at most `max` are left.
fn join_boxes(boxes: Vec<[f32; 4]>, max: usize) -> Vec<[f32; 4]> {
    let mut b: Vec<[f32; 4]> = boxes.into_iter().filter(|b| b[0] <= b[2])
        .map(|b| [(b[0] - 1.0).floor(), (b[1] - 1.0).floor(), (b[2] + 1.0).ceil(), (b[3] + 1.0).ceil()]).collect();
    let union = |a: &[f32; 4], c: &[f32; 4]| [a[0].min(c[0]), a[1].min(c[1]), a[2].max(c[2]), a[3].max(c[3])];
    let area = |a: &[f32; 4]| (a[2] - a[0]) * (a[3] - a[1]);
    // Many boxes (a scroll past the held content, a big change): first gathered on a coarse grid (256 px cells, each
    // box into the cell of its top-left corner), so the pairwise joining below sees a few dozen.
    if b.len() > 48 {
        let mut cells: std::collections::HashMap<(i32, i32), [f32; 4]> = std::collections::HashMap::new();
        for x in &b {
            let k = ((x[0] / 256.0).floor() as i32, (x[1] / 256.0).floor() as i32);
            cells.entry(k).and_modify(|c| *c = union(c, x)).or_insert(*x);
        }
        b = cells.into_values().collect();
        while b.len() > 48 {
            // Still many: halve them by joining neighbours in reading order.
            b.sort_by(|p, q| (p[1], p[0]).partial_cmp(&(q[1], q[0])).unwrap());
            b = b.chunks(2).map(|c| if c.len() == 2 { union(&c[0], &c[1]) } else { c[0] }).collect();
        }
    }
    loop {
        let n = b.len();
        if n <= 1 { break; }
        let mut best: Option<(usize, usize, f32)> = None;
        for i in 0..n {
            for j in i + 1..n {
                let u = union(&b[i], &b[j]);
                let overlap = b[i][0] <= b[j][2] && b[j][0] <= b[i][2] && b[i][1] <= b[j][3] && b[j][1] <= b[i][3];
                let growth = if overlap { -1.0 } else { area(&u) - area(&b[i]) - area(&b[j]) };
                if best.is_none_or(|x| growth < x.2) { best = Some((i, j, growth)); }
            }
        }
        let (i, j, growth) = best.unwrap();
        if n <= max && growth >= 0.0 { break; }
        b[i] = union(&b[i], &b[j]);
        b.swap_remove(j);
    }
    b
}

/// FNV-1a over 64-bit words: quick, and good enough to tell shapes apart.
struct Key(u64);
impl Key {
    fn new() -> Self { Key(0xcbf29ce484222325) }
    fn word(&mut self, w: u64) -> &mut Self { for b in w.to_le_bytes() { self.0 = (self.0 ^ b as u64).wrapping_mul(0x100000001b3); } self }
    fn f(&mut self, v: f64) -> &mut Self { self.word(v.to_bits()) }
    fn path(&mut self, p: &BezPath) -> &mut Self {
        for el in p.elements() {
            match *el {
                PathEl::MoveTo(a) => { self.word(1).f(a.x).f(a.y); }
                PathEl::LineTo(a) => { self.word(2).f(a.x).f(a.y); }
                PathEl::QuadTo(c, a) => { self.word(3).f(c.x).f(c.y).f(a.x).f(a.y); }
                PathEl::CurveTo(c1, c2, a) => { self.word(4).f(c1.x).f(c1.y).f(c2.x).f(c2.y).f(a.x).f(a.y); }
                PathEl::ClosePath => { self.word(5); }
            }
        }
        self
    }
    fn affine(&mut self, t: Affine) -> &mut Self { for c in t.as_coeffs() { self.f(c); } self }
}

impl TessCache {
    /// `rounded_rect`, or failing that `hole_clip`, remembered by outline and transform.
    fn clip_shape(&mut self, shape: &BezPath, t: Affine) -> Option<[f32; 12]> {
        let key = Key::new().word(77).affine(t).path(shape).0;
        let g = self.generation;
        if let Some(e) = self.clip_shapes.get_mut(&key) { e.1 = g; return e.0; }
        let r = rounded_rect(shape, t).or_else(|| hole_clip(shape, t));
        self.clip_shapes.insert(key, (r, g));
        r
    }

    /// The cached triangles for `key`, or `make`'s.
    fn get(&mut self, key: u64, make: impl FnOnce() -> Geo) -> &Geo {
        let generation = self.generation;
        let misses = &mut self.misses;
        let e = self.map.entry(key).or_insert_with(|| { *misses += 1; (make(), generation) });
        e.1 = generation;
        &e.0
    }
    fn begin(&mut self) { self.generation = self.generation.wrapping_add(1); self.misses = 0; }
    /// Drops shapes not drawn in the last 120 frames (a hover state that comes back soon stays).
    fn end(&mut self) {
        let g = self.generation;
        self.map.retain(|_, e| g.wrapping_sub(e.1) < 120);
        self.clip_shapes.retain(|_, e| g.wrapping_sub(e.1) < 120);
        if self.ramps.len() > 4096 { self.ramps.clear(); }
        self.groups.retain(|_, e| e.used == g);
    }
}

/// A transform without its move, and the move: shapes are tessellated and cached at the origin, so one that only moves
/// (a sliding panel, a scrolled list, a translate animation) is never tessellated again.
fn at_origin(t: Affine) -> (Affine, [f32; 2]) {
    let c = t.as_coeffs();
    (Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0]), [c[4] as f32, c[5] as f32])
}

/// Cached triangles (made at the origin) with a brush, moved by `off`: each vertex painted by `bm`, its coverage times
/// `cov` folded into the colour.
fn painted_at(geo: &Geo, bm: &BrushMap, cov: f32, off: [f32; 2]) -> VertexBuffers<Vertex, u32> {
    let mut out: VertexBuffers<Vertex, u32> = VertexBuffers::with_capacity(geo.vertices.len(), geo.indices.len());
    out.vertices.extend(geo.vertices.iter().map(|&[x, y, c]| {
        let mut v = bm.vertex(x + off[0], y + off[1]);
        let k = c * cov;
        if k < 1.0 { v.color = scale_color(v.color, k); }
        v
    }));
    out.indices.extend_from_slice(&geo.indices);
    out
}

/// A filled shape with analytic edge antialiasing (as NanoVG and Skia's GPU path do), so no multisampling is needed:
/// the outline is flattened, its inside drawn inset by half a pixel at full coverage, and a one-pixel fringe around it
/// fades from full coverage to none. Edges cost two triangles per segment; the fill's own triangles are unchanged.
fn aa_fill(fill: &mut FillTessellator, shape: &BezPath, t: Affine, rule: peniko::Fill) -> Geo { aa_fill_grown(fill, shape, t, rule, 0.0) }

/// `aa_fill` with the outline moved outward by `grow` device px (synthetic emboldening of text).
fn aa_fill_grown(fill: &mut FillTessellator, shape: &BezPath, t: Affine, rule: peniko::Fill, grow: f32) -> Geo {
    let mut geo: Geo = VertexBuffers::new();
    // The outline as polygons in device px.
    let mut polys: Vec<Vec<(f32, f32)>> = Vec::new();
    let moved = t * shape.clone();
    peniko::kurbo::flatten(moved.elements().iter().copied(), TOLERANCE as f64, |el| match el {
        PathEl::MoveTo(a) => polys.push(vec![(a.x as f32, a.y as f32)]),
        PathEl::LineTo(a) => {
            if polys.is_empty() { polys.push(Vec::new()); }
            let poly = polys.last_mut().unwrap();
            let q = (a.x as f32, a.y as f32);
            if poly.last().is_none_or(|l| (l.0 - q.0).abs() + (l.1 - q.1).abs() > 1e-4) { poly.push(q); }
        }
        _ => {}
    });
    for poly in &mut polys {
        while poly.len() > 1 && { let (f, l) = (poly[0], poly[poly.len() - 1]); (f.0 - l.0).abs() + (f.1 - l.1).abs() <= 1e-4 } { poly.pop(); }
    }
    polys.retain(|p| p.len() >= 3);
    if polys.is_empty() { return geo; }
    // Which side is outside: the whole shape's orientation (holes run the other way under the non-zero rule).
    let area: f32 = polys.iter().map(|p| (0..p.len()).map(|i| { let (a, b) = (p[i], p[(i + 1) % p.len()]); a.0 * b.1 - b.0 * a.1 }).sum::<f32>()).sum();
    let side = if area >= 0.0 { 1.0 } else { -1.0 };
    let mut inner = Path::builder();
    for p in &polys {
        let n = p.len();
        // Each edge's outward unit normal.
        let normals: Vec<(f32, f32)> = (0..n).map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l = (dx * dx + dy * dy).sqrt().max(1e-6);
            (side * dy / l, -side * dx / l)
        }).collect();
        let base = geo.vertices.len() as u32;
        for i in 0..n {
            // The corner's offset: both edges move by half a pixel (a miter, limited at sharp corners).
            let (n0, n1) = (normals[(i + n - 1) % n], normals[i]);
            let (mx, my) = ((n0.0 + n1.0) * 0.5, (n0.1 + n1.1) * 0.5);
            let k = 1.0 / (mx * mx + my * my).max(0.25);
            let (ux, uy) = (mx * k, my * k);
            let (x, y) = p[i];
            let (ix, iy) = (x + ux * (grow - 0.5), y + uy * (grow - 0.5));
            geo.vertices.push([ix, iy, 1.0]);
            geo.vertices.push([x + ux * (grow + 0.5), y + uy * (grow + 0.5), 0.0]);
            if i == 0 { inner.begin(point(ix, iy)); } else { inner.line_to(point(ix, iy)); }
        }
        inner.end(true);
        for i in 0..n as u32 {
            let j = (i + 1) % n as u32;
            let (a, b, c, d) = (base + 2 * i, base + 2 * i + 1, base + 2 * j + 1, base + 2 * j);
            geo.indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    let rule = match rule { peniko::Fill::EvenOdd => FillRule::EvenOdd, _ => FillRule::NonZero };
    let _ = fill.tessellate_path(&inner.build(), &FillOptions::tolerance(TOLERANCE).with_fill_rule(rule),
        &mut BuffersBuilder::new(&mut geo, |v: FillVertex| [v.position().x, v.position().y, 1.0]));
    geo
}

impl Frame {
    /// The open shader clips packed for `Vertex::meta` (above the material's eight bits) and `Vertex::meta2`.
    fn clip_meta(&self) -> (u32, u32) { ((self.slots[0] << 8) | (self.scroll << 24), self.slots[1] | (self.slots[2] << 16)) }

    /// Opens a shader clip in a free slot; the slot, or None when all three are taken or the table is full.
    fn open_clip(&mut self, rr: [f32; 12]) -> Option<usize> {
        let slot = self.slots.iter().position(|&c| c == 0)?;
        let mut row16 = [0f32; 16];
        row16[..12].copy_from_slice(&rr);
        row16[12] = self.scroll as f32;
        let mut k = Key::new();
        for v in row16 { k.f(v as f64); }
        let row = self.clip_table.get(k.0, MAX_CLIPS, || row16)?;
        for r in &mut self.recording { r.clips.push(k.0); }
        self.slots[slot] = row + 1;
        Some(slot)
    }

    /// Adds geometry drawn with a material; joins the last draw when nothing came between and the material is the same.
    fn push(&mut self, kind: u32, texture: u32, _stencil: u32, geo: VertexBuffers<Vertex, u32>) {
        if geo.indices.is_empty() { return; }
        // Every draw goes through the uber material: its material and clip ride on the vertices, so consecutive draws
        // join one batch whatever they are, and only a texture of their own (a big or repeating image) breaks it.
        let (m1, meta2) = self.clip_meta();
        let meta = kind | m1;
        let tagged: Vec<Vertex> = geo.vertices.iter().map(|v| Vertex { meta, meta2, ..*v }).collect();
        let (base, key) = self.pool.place_keyed(&mut self.vertices, &tagged);
        for r in &mut self.recording {
            if texture != 0 { r.draws = None; continue; }
            if let Some(d) = &mut r.draws { d.push(GroupDraw { texture, at: base, n: tagged.len() as u32, key, indices: geo.indices.clone() }); }
        }
        self.push_indices(texture, base, &geo.indices);
    }

    /// Appends a draw's indices (its vertices at `base`), joining the last batch when nothing came between.
    fn push_indices(&mut self, texture: u32, base: u32, indices: &[u32]) {
        let (kind, stencil) = (UBER, 0);
        let first = self.indices.len() as u32;
        self.indices.extend(indices.iter().map(|i| i + base));
        let count = indices.len() as u32;
        let joinable = matches!(self.ops.last(), Some(op) if op.kind == 0)
            && matches!(self.batches.last(), Some(b) if b.kind == kind && b.texture == texture && b.stencil == stencil && b.first + b.count == first);
        if joinable {
            self.batches.last_mut().unwrap().count += count;
        } else {
            self.batches.push(Batch { kind, first, count, texture, stencil });
            self.ops.push(Op { kind: 0, batch: self.batches.len() as u32 - 1, ..Default::default() });
        }
    }

    /// Adds a clip shape as its own batch and returns its index (empty shapes still get one, so pushes and pops pair).
    fn clip(&mut self, geo: VertexBuffers<Vertex, u32>) -> u32 {
        let first = self.indices.len() as u32;
        // The shape a group is composited through is also cut by the shader clips open around it.
        let (meta, meta2) = self.clip_meta();
        let tagged: Vec<Vertex> = geo.vertices.iter().map(|v| Vertex { meta, meta2, ..*v }).collect();
        let base = if tagged.is_empty() { 0 } else { self.pool.place(&mut self.vertices, &tagged) };
        self.indices.extend(geo.indices.iter().map(|i| i + base));
        self.batches.push(Batch { kind: CLIP, first, count: geo.indices.len() as u32, texture: 0, stencil: 0 });
        self.batches.len() as u32 - 1
    }
}

/// Whether a clip is an axis-aligned rectangle (a stencil cuts it; no soft edge needed).
fn pixel_rect(shape: &BezPath, t: Affine) -> bool {
    let c = t.as_coeffs();
    if c[1].abs() > 1e-6 || c[2].abs() > 1e-6 { return false; }
    let mut pts = Vec::with_capacity(5);
    for el in shape.elements() {
        match *el {
            PathEl::MoveTo(a) | PathEl::LineTo(a) => pts.push(t * a),
            PathEl::ClosePath => {}
            _ => return false,
        }
    }
    if pts.len() == 5 && (pts[0] - pts[4]).hypot() < 1e-6 { pts.pop(); }
    if pts.len() != 4 { return false; }
    // Any axis-aligned rectangle: on whole pixels the stencil cuts it exactly; off them it cuts within half a pixel,
    // which costs far less than making every scaled panel and viewport its own texture.
    (0..4).all(|i| { let (a, b) = (pts[i], pts[(i + 1) % 4]); (a.x - b.x).abs() < 1e-6 || (a.y - b.y).abs() < 1e-6 })
}

/// A glyph's outline in px at the run's size (y down, origin on the baseline), or None when it has none.
fn glyph_outline(run: &anyrender::recording::GlyphRunCommand, glyph: u32) -> Option<BezPath> {
    use skrifa::MetadataProvider;
    use skrifa::outline::{DrawSettings, OutlinePen};
    struct Pen(BezPath);
    impl OutlinePen for Pen {
        fn move_to(&mut self, x: f32, y: f32) { self.0.move_to((x as f64, -y as f64)); }
        fn line_to(&mut self, x: f32, y: f32) { self.0.line_to((x as f64, -y as f64)); }
        fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) { self.0.quad_to((cx as f64, -cy as f64), (x as f64, -y as f64)); }
        fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
            self.0.curve_to((c0x as f64, -c0y as f64), (c1x as f64, -c1y as f64), (x as f64, -y as f64));
        }
        fn close(&mut self) { self.0.close_path(); }
    }
    let font = skrifa::FontRef::from_index(run.font_data.data.data(), run.font_data.index).ok()?;
    let outline = font.outline_glyphs().get(skrifa::GlyphId::new(glyph))?;
    let coords: Vec<skrifa::instance::NormalizedCoord> = run.normalized_coords.iter().map(|&c| skrifa::instance::NormalizedCoord::from_bits(c)).collect();
    let mut pen = Pen(BezPath::new());
    outline.draw(DrawSettings::unhinted(skrifa::instance::Size::new(run.font_size), skrifa::instance::LocationRef::new(&coords)), &mut pen).ok()?;
    if pen.0.elements().is_empty() { None } else { Some(pen.0) }
}

/// A clip shape as a rounded rectangle in device px (x0 y0 x1 y1 and rx ry per corner: top-left, top-right,
/// bottom-right, bottom-left) when it is one under an axis-aligned transform: the shaders then clip per pixel instead of
/// drawing through an offscreen texture. Found from the outline's bounds and where its sides start, then checked: every
/// point of the flattened outline must lie on that rounded rectangle.
fn rounded_rect(shape: &BezPath, t: Affine) -> Option<[f32; 12]> {
    let c = t.as_coeffs();
    if c[1].abs() > 1e-6 || c[2].abs() > 1e-6 { return None; }
    let path = t * shape.clone();
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let mut subpaths = 0;
    peniko::kurbo::flatten(path.elements().iter().copied(), 0.05, |el| match el {
        PathEl::MoveTo(p) => { subpaths += 1; pts.push((p.x, p.y)); }
        PathEl::LineTo(p) => pts.push((p.x, p.y)),
        _ => {}
    });
    if subpaths != 1 || pts.len() < 4 { return None; }
    let (x0, y0, x1, y1) = pts.iter().fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |a, p| (a.0.min(p.0), a.1.min(p.1), a.2.max(p.0), a.3.max(p.1)));
    if x1 - x0 < 1.0 || y1 - y0 < 1.0 { return None; }
    let e = 0.02;
    // Where each side's straight part ends: the extreme points lying on that side.
    let on = |f: &dyn Fn(&(f64, f64)) -> bool| pts.iter().filter(|p| f(p)).copied().collect::<Vec<_>>();
    let top = on(&|p| (p.1 - y0).abs() < e);
    let bottom = on(&|p| (p.1 - y1).abs() < e);
    let left = on(&|p| (p.0 - x0).abs() < e);
    let right = on(&|p| (p.0 - x1).abs() < e);
    if top.is_empty() || bottom.is_empty() || left.is_empty() || right.is_empty() { return None; }
    let min_x = |v: &[(f64, f64)]| v.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    let max_x = |v: &[(f64, f64)]| v.iter().map(|p| p.0).fold(f64::MIN, f64::max);
    let min_y = |v: &[(f64, f64)]| v.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    let max_y = |v: &[(f64, f64)]| v.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    let r = [
        (min_x(&top) - x0, min_y(&left) - y0),      // top-left
        (x1 - max_x(&top), min_y(&right) - y0),     // top-right
        (x1 - max_x(&bottom), y1 - max_y(&right)),  // bottom-right
        (min_x(&bottom) - x0, y1 - max_y(&left)),   // bottom-left
    ];
    // Every outline point on the boundary (within a tenth of a pixel).
    let sdf = |px: f64, py: f64| -> f64 {
        let (cx, cy, rx, ry) = if px < x0 + r[0].0 && py < y0 + r[0].1 { (x0 + r[0].0, y0 + r[0].1, r[0].0, r[0].1) }
            else if px > x1 - r[1].0 && py < y0 + r[1].1 { (x1 - r[1].0, y0 + r[1].1, r[1].0, r[1].1) }
            else if px > x1 - r[2].0 && py > y1 - r[2].1 { (x1 - r[2].0, y1 - r[2].1, r[2].0, r[2].1) }
            else if px < x0 + r[3].0 && py > y1 - r[3].1 { (x0 + r[3].0, y1 - r[3].1, r[3].0, r[3].1) }
            else { return (x0 - px).max(px - x1).max(y0 - py).max(py - y1); };
        if rx < 1e-3 || ry < 1e-3 { return (x0 - px).max(px - x1).max(y0 - py).max(py - y1); }
        let (qx, qy) = ((px - cx) / rx, (py - cy) / ry);
        ((qx * qx + qy * qy).sqrt() - 1.0) * rx.min(ry)
    };
    // Checked along every segment, not only at its ends (a diamond's corners lie on the inscribed ellipse).
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        let steps = (((b.0 - a.0).abs() + (b.1 - a.1).abs()) / 2.0).ceil().max(1.0) as usize;
        for k in 0..steps {
            let f = k as f64 / steps as f64;
            if sdf(a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f).abs() > 0.1 { return None; }
        }
    }
    let f = |v: f64| v as f32;
    Some([f(x0), f(y0), f(x1), f(y1), f(r[0].0), f(r[0].1), f(r[1].0), f(r[1].1), f(r[2].0), f(r[2].1), f(r[3].0), f(r[3].1)])
}

/// A clip that is everything but a rounded rectangle (an outer box-shadow's clip: a big rectangle and the element's
/// rounded border box as a hole), as an inverted shader clip: the rounded rectangle with x0 and x1 swapped. The big
/// rectangle is dropped: what is drawn under such a clip lies within it.
fn hole_clip(shape: &BezPath, t: Affine) -> Option<[f32; 12]> {
    let mut subs: Vec<BezPath> = Vec::new();
    for el in shape.elements() {
        if matches!(el, PathEl::MoveTo(_)) { subs.push(BezPath::new()); }
        subs.last_mut()?.push(*el);
    }
    if subs.len() != 2 { return None; }
    let outer = rounded_rect(&subs[0], t)?;
    if outer[4..].iter().any(|&r| r > 0.01) { return None; }
    let mut inner = rounded_rect(&subs[1], t)?;
    if inner[0] < outer[0] || inner[1] < outer[1] || inner[2] > outer[2] || inner[3] > outer[3] { return None; }
    inner.swap(0, 2);
    Some(inner)
}

/// The hole of an inset box-shadow: blurred (a box shadow) or sharp (a filled rounded rectangle, when the shadow has
/// no blur).
enum InsetHole<'a> { Blurred(&'a anyrender::recording::BoxShadowCommand), Sharp(&'a anyrender::recording::FillCommand) }

/// The six commands of an inset box-shadow as Blitz records it, when `cmds` starts with them: the padding-box layer, its
/// colour (premultiplied), and the hole cut out of it.
fn inset_shadow<'a>(cmds: &[&'a RenderCommand]) -> Option<(&'a anyrender::recording::LayerCommand, u32, InsetHole<'a>)> {
    let [RenderCommand::PushLayer(l1), RenderCommand::Fill(f), RenderCommand::PushLayer(l2), hole, RenderCommand::PopLayer, RenderCommand::PopLayer, ..] = cmds else { return None };
    let plain = |l: &anyrender::recording::LayerCommand| l.alpha == 1.0 && l.filter.is_none() && l.backdrop_filter.is_none() && l.blend.mix == peniko::Mix::Normal;
    if !plain(l1) || !plain(l2) || l1.blend.compose != peniko::Compose::SrcOver || l2.blend.compose != peniko::Compose::DestOut { return None; }
    if f.shape != l1.clip || l2.clip != l1.clip || f.transform != l1.transform || f.brush_transform.is_some() { return None; }
    let Paint::Solid(color) = &f.brush else { return None };
    let hole = match hole {
        RenderCommand::BoxShadow(b) if b.brush.components[3] >= 0.999 => InsetHole::Blurred(b),
        RenderCommand::Fill(h) if matches!(&h.brush, Paint::Solid(c) if c.components[3] >= 0.999) => InsetHole::Sharp(h),
        _ => return None,
    };
    Some((l1, premul(*color, 1.0), hole))
}

/// Builds the frame for a page's draw commands.
pub fn build(page: &Scene, glyphs: &mut GlyphCache, aa: bool, last_vertices: Vec<Vertex>) -> Frame {
    // Shared pieces (elements painted earlier) written out in place by reference, each remembering its span: an
    // unchanged one appends what it drew last time instead of being walked.
    fn flat<'a>(cmds: &'a [RenderCommand], out: &mut Vec<&'a RenderCommand>, spans: &mut Vec<(usize, usize, &'a std::sync::Arc<Vec<RenderCommand>>)>) {
        for c in cmds {
            if let RenderCommand::Group(g) = c { let start = out.len(); flat(g, out, spans); spans.push((start, out.len(), g)); } else { out.push(c); }
        }
    }
    let mut refs: Vec<&RenderCommand> = Vec::with_capacity(page.commands.len() * 4);
    let mut spans = Vec::new();
    flat(&page.commands, &mut refs, &mut spans);
    // Spans by where they start, outermost first.
    let mut starts: std::collections::HashMap<usize, Vec<(usize, &std::sync::Arc<Vec<RenderCommand>>)>> = std::collections::HashMap::new();
    for &(a, b, g) in spans.iter().rev() { if b > a { starts.entry(a).or_default().push((b, g)); } }
    let mut frame = Frame::default();
    glyphs.tess.begin();
    let mut tess = std::mem::take(&mut glyphs.tess);
    // The vertex buffer goes on from the last frame's: unchanged draws keep their places.
    frame.vertices = last_vertices;
    frame.pool = std::mem::take(&mut tess.pool);
    frame.pool.begin(&mut frame.vertices);
    frame.clip_table = std::mem::take(&mut tess.clip_table);
    frame.clip_table.begin();
    tess.ramp_table.begin();
    let mut fill = FillTessellator::new();
    let mut stroke = StrokeTessellator::new();
    // Each open layer: whether it was pushed as a group (true) or a clip, and its clip batch.
    // Each open layer: group or stencil clip, its clip batch, how it composites, its filter steps, and the shader clip
    // slot it took (Some when the layer was a shader clip, which pushes no step).
    let mut stack: Vec<(bool, u32, f32, u32, u32, u32, u32, Option<u32>)> = Vec::new();
    // The composited layer being drawn: its index, the first vertex it drew and its marker depth.
    let mut open_layer: Option<(usize, usize, i32)> = None;
    let mut marker_depth = 0i32;
    let mut scroll_stack: Vec<u32> = Vec::new();
    // A clip's shape. `soft`: with antialiased edges, for a group composited through it (a stencil can only cut hard).
    let tess_clip = |frame: &mut Frame, tess: &mut TessCache, fill: &mut FillTessellator, shape: &BezPath, t: Affine, rule: peniko::Fill, soft: bool| -> u32 {
        let (t, off) = at_origin(t);
        let key = Key::new().word(20 + soft as u64).word(rule as u64).affine(t).path(shape).0;
        let geo = tess.get(key, || {
            if soft { return aa_fill(fill, shape, t, rule); }
            let mut geo: Geo = VertexBuffers::new();
            let rule = match rule { peniko::Fill::EvenOdd => FillRule::EvenOdd, _ => FillRule::NonZero };
            let _ = fill.tessellate_path(&to_lyon(shape, t), &FillOptions::tolerance(TOLERANCE).with_fill_rule(rule),
                &mut BuffersBuilder::new(&mut geo, |v: FillVertex| [v.position().x, v.position().y, 1.0]));
            geo
        });
        let mut out: VertexBuffers<Vertex, u32> = VertexBuffers::with_capacity(geo.vertices.len(), geo.indices.len());
        out.vertices.extend(geo.vertices.iter().map(|&[x, y, c]| Vertex { x: x + off[0], y: y + off[1], color: scale_color(0xffffffff, c), ..Default::default() }));
        out.indices.extend_from_slice(&geo.indices);
        frame.clip(out)
    };
    // A layer that holds a layer drawn with another operator than "over" (an inset shadow cut out with DestOut, a mask
    // with DestIn) must be its own texture too: the operator may only touch what that layer drew, as in the CPU
    // painter, never what lies under it.
    let mut isolate = vec![false; refs.len()];
    {
        let mut open: Vec<usize> = Vec::new();
        for (i, cmd) in refs.iter().enumerate() {
            match cmd {
                RenderCommand::PushLayer(l) => {
                    // Only the layer it sits in must be its own texture: the operator then touches just what that
                    // layer drew. (Isolating every ancestor made the whole page one big group.)
                    if l.blend.compose != peniko::Compose::SrcOver || l.blend.mix != peniko::Mix::Normal {
                        if let Some(&o) = open.last() { isolate[o] = true; }
                    }
                    open.push(i);
                }
                RenderCommand::PushClipLayer(_) => open.push(i),
                RenderCommand::PopLayer => { open.pop(); }
                _ => {}
            }
        }
    }
    let mut skip_to = 0usize;
    let epoch = glyphs.epoch;
    let mut ops_seen = 0usize;
    for index in 0..=refs.len() {
        // The last command took a real step (a stencil clip, a group, a backdrop, a layer): groups recording around
        // it can't be replayed by appending draws. Shader clips take no step and are replayed with their rows.
        if frame.ops[ops_seen..].iter().any(|o| o.kind != 0) { for r in &mut frame.recording { r.draws = None; } }
        ops_seen = frame.ops.len();
        // Groups whose span ends here: what they drew is kept for the next frame.
        while frame.recording.last().is_some_and(|r| r.end == index) {
            let r = frame.recording.pop().unwrap();
            if let Some(draws) = r.draws {
                tess.groups.insert(r.ptr, GroupDraws { _piece: r.piece, context: r.context, draws, ramps: r.ramps, clips: r.clips, used: tess.generation });
            }
        }
        if index == refs.len() { break; }
        if index < skip_to { continue; }
        let cmd = refs[index];
        // A shared piece starts here: replayed when drawn last time in the same context, else recorded as it is walked.
        if let Some(list) = starts.get(&index) {
            let (m1, m2) = frame.clip_meta();
            let context = (m1, m2, epoch);
            // Under clips and groups a replay appends the same draws in the same order (the shader clips are part of
            // the context); composited layers keep their own vertex ranges, so not inside one.
            let plain = open_layer.is_none() && std::env::var_os("VITRAIL_NO_GROUPS").is_none();
            let mut replayed = false;
            for &(end, g) in list {
                let ptr = std::sync::Arc::as_ptr(g) as usize;
                if plain && !replayed {
                    if let Some(gd) = tess.groups.get_mut(&ptr) {
                        if gd.context == context
                            && gd.draws.iter().all(|d| frame.pool.keep(d.key, d.at, d.n))
                            && gd.ramps.iter().all(|&k| tess.ramp_table.touch(k))
                            && gd.clips.iter().all(|&k| frame.clip_table.touch(k))
                        {
                            gd.used = tess.generation;
                            for d in &gd.draws { frame.push_indices(d.texture, d.at, &d.indices); }
                            // Outer groups being recorded get these draws too.
                            for r in &mut frame.recording { if let Some(dr) = &mut r.draws { dr.extend(gd.draws.iter().cloned()); } r.ramps.extend(gd.ramps.iter().copied()); r.clips.extend(gd.clips.iter().copied()); }
                            skip_to = end;
                            replayed = true;
                            break;
                        }
                    }
                }
                frame.recording.push(GroupRecording { end, ptr, piece: g.clone(), context, draws: plain.then(Vec::new), ramps: Vec::new(), clips: Vec::new() });
            }
            // Spans inside a replayed one were never pushed (the loop broke); outer ones go on recording.
            if replayed { continue; }
        }
        // Composited layers and scroll containers keep their own records: a group holding one isn't replayed.
        if !frame.recording.is_empty() && matches!(cmd, RenderCommand::CompositedBegin(_) | RenderCommand::CompositedEnd | RenderCommand::ScrollBegin(_) | RenderCommand::ScrollEnd) {
            for r in &mut frame.recording { r.draws = None; }
        }
        // An inset box-shadow as Blitz draws it (a layer clipped to the padding box: its colour, then the blurred hole
        // cut out with DestOut) becomes one draw: the colour where the hole doesn't cover, in the padding box's shape.
        if aa {
            if let Some((l, color, hole)) = inset_shadow(&refs[index..]) {
                let (lin, off) = at_origin(l.transform);
                let key = Key::new().word(1).word(l.fill as u64).affine(lin).path(&l.clip).0;
                match hole {
                    InsetHole::Blurred(b) => {
                        let geo = tess.get(key, || aa_fill(&mut fill, &l.clip, lin, l.fill));
                        let inv = b.transform.inverse();
                        let rect = b.rect.abs();
                        let p = [rect.width() as f32, rect.height() as f32, b.radius as f32, b.std_dev as f32];
                        let mut out: VertexBuffers<Vertex, u32> = VertexBuffers::with_capacity(geo.vertices.len(), geo.indices.len());
                        out.vertices.extend(geo.vertices.iter().map(|&[x, y, cov]| {
                            let (dx, dy) = (x + off[0], y + off[1]);
                            let q = inv * peniko::kurbo::Point::new(dx as f64, dy as f64);
                            Vertex { x: dx, y: dy, color: scale_color(color, cov), u: (q.x - rect.x0) as f32, v: (q.y - rect.y0) as f32, p, ..Default::default() }
                        }));
                        out.indices.extend_from_slice(&geo.indices);
                        frame.push(INSET_SHADOW, 0, 0, out);
                        skip_to = index + 6;
                        continue;
                    }
                    // A sharp hole: the colour in the padding box under an inverted shader clip of the hole.
                    InsetHole::Sharp(h) if frame.slots.contains(&0) => {
                        if let Some(mut rr) = tess.clip_shape(&h.shape, h.transform) {
                            rr.swap(0, 2);
                            let geo = tess.get(key, || aa_fill(&mut fill, &l.clip, lin, l.fill));
                            let Some(slot) = frame.open_clip(rr) else { continue };
                            let bm = BrushMap { kind: SOLID, texture: 0, color, inv: Affine::IDENTITY, map: |_, _| ([0.0; 2], [0.0; 4]), p: [0.0; 4], origin: Default::default(), dir: Default::default(), size: [1.0, 1.0], q: [0.0; 4] };
                            frame.push(SOLID, 0, 0, painted_at(geo, &bm, 1.0, off));
                            frame.slots[slot] = 0;
                            skip_to = index + 6;
                            continue;
                        }
                    }
                    _ => {}
                }
            }
        }
        match cmd {
            // Written out in place before the walk (`build`'s start).
            RenderCommand::Group(_) => {}
            RenderCommand::PushLayer(l) => {
                if let Some(bf) = &l.backdrop_filter {
                    // backdrop-filter: the steps run within the element's bounds (the blur repeats the edge pixels
                    // rather than pulling in colours from beyond, as Chrome does), then come back through its shape.
                    let bb = peniko::kurbo::Shape::bounding_box(&(l.transform * l.clip.clone()));
                    let first = frame.filters.len() as u32;
                    filter_steps(bf, l.transform, [0.0; 4], &mut frame.filters);
                    if frame.filters.len() as u32 == first {
                        // No step it can run (or none at all): an identity matrix, so the bounds still travel.
                        let mut st = FilterStep { kind: 3, ..Default::default() };
                        (st.m[0], st.m[6], st.m[12], st.m[18]) = (1.0, 1.0, 1.0, 1.0);
                        frame.filters.push(st);
                    }
                    let bounds = [bb.x0 as f32, bb.y0 as f32, bb.x1 as f32, bb.y1 as f32];
                    for st in &mut frame.filters[first as usize..] { st.bounds = bounds; }
                    let shape = tess_clip(&mut frame, &mut tess, &mut fill, &l.clip, l.transform, l.fill, aa);
                    frame.ops.push(Op { kind: 5, batch: shape, filter: first, filter_count: frame.filters.len() as u32 - first, alpha: 1.0, compose: peniko::Compose::SrcOver as u32, mix: 0 });
                }
                // A layer needs its own texture when it fades, blends other than plainly, filters, or holds such a
                // layer; otherwise it is only a clip. With soft edges a clip that is not a whole-pixel rectangle is
                // a texture too, composited through its antialiased shape.
                let plain = l.blend.compose == peniko::Compose::SrcOver && l.blend.mix == peniko::Mix::Normal;
                // Blend modes are drawn by a pass that weighs the result by coverage, so they take soft edges too.
                let soft = aa && l.blend.compose == peniko::Compose::SrcOver && !pixel_rect(&l.clip, l.transform);
                let group = soft || l.alpha < 1.0 || !plain || l.filter.is_some() || isolate[index];
                // A layer that only clips, to a rectangle or a rounded one (or everything but one), is clipped by the
                // shaders while a slot is free: no stencil step, no texture, the batch goes on.
                if !(l.alpha < 1.0 || !plain || l.filter.is_some() || isolate[index]) {
                    if let Some(slot) = tess.clip_shape(&l.clip, l.transform).and_then(|rr| frame.open_clip(rr)) {
                        stack.push((false, 0, 1.0, 0, 0, 0, 0, Some(slot as u32)));
                        continue;
                    }
                }
                let clip = tess_clip(&mut frame, &mut tess, &mut fill, &l.clip, l.transform, l.fill, soft);
                frame.ops.push(Op { kind: if group { 3 } else { 1 }, batch: clip, ..Default::default() });
                let first = frame.filters.len() as u32;
                if let Some(f) = &l.filter {
                    let bb = peniko::kurbo::Shape::bounding_box(&(l.transform * l.clip.clone()));
                    filter_steps(f, l.transform, [bb.x0 as f32, bb.y0 as f32, bb.x1 as f32, bb.y1 as f32], &mut frame.filters);
                }
                stack.push((group, clip, l.alpha, l.blend.compose as u32, l.blend.mix as u32, first, frame.filters.len() as u32 - first, None));
            }
            RenderCommand::PushClipLayer(c) => {
                let soft = aa && !pixel_rect(&c.clip, c.transform);
                let group = soft || isolate[index];
                if !isolate[index] {
                    if let Some(slot) = tess.clip_shape(&c.clip, c.transform).and_then(|rr| frame.open_clip(rr)) {
                        stack.push((false, 0, 1.0, 0, 0, 0, 0, Some(slot as u32)));
                        continue;
                    }
                }
                let clip = tess_clip(&mut frame, &mut tess, &mut fill, &c.clip, c.transform, c.fill, soft);
                frame.ops.push(Op { kind: if group { 3 } else { 1 }, batch: clip, ..Default::default() });
                stack.push((group, clip, 1.0, peniko::Compose::SrcOver as u32, 0, 0, 0, None));
            }
            RenderCommand::PopLayer => {
                if let Some((group, clip, alpha, compose, mix, filter, filter_count, shader)) = stack.pop() {
                    match shader {
                        Some(slot) => frame.slots[slot as usize] = 0,
                        None => frame.ops.push(Op { kind: if group { 4 } else { 2 }, batch: clip, alpha, compose, mix, filter, filter_count }),
                    }
                }
            }
            RenderCommand::ScrollBegin(c) => {
                // Content painted unscrolled: its vertices and clips carry this container, the GPU moves them.
                scroll_stack.push(frame.scroll);
                if frame.scrolls.len() < MAX_SCROLLS {
                    let cl = c.clip;
                    frame.scrolls.push(ScrollInfo { id: c.id, parent: frame.scroll, _pad: 0, offset: [c.offset.x as f32, c.offset.y as f32], clip: [cl.x0 as f32, cl.y0 as f32, cl.x1 as f32, cl.y1 as f32] });
                    frame.scroll = frame.scrolls.len() as u32;
                }
            }
            RenderCommand::ScrollEnd => { frame.scroll = scroll_stack.pop().unwrap_or(0); }
            RenderCommand::CompositedBegin(c) => {
                // Composited on its own only where nothing but stencil clips is open around it (a group would have to
                // be redrawn anyway), not inside another composited element and not inside scrolled content;
                // otherwise it is drawn with the page.
                marker_depth += 1;
                let inline = open_layer.is_some() || stack.iter().any(|e| e.0) || frame.scroll != 0;
                let index = frame.layers.len();
                frame.layers.push(CompLayer { id: c.id, inline: inline as u32, _pad: 0, world: coeffs(c.world), local: coeffs(c.local), bounds: [0.0; 4] });
                if !inline {
                    frame.ops.push(Op { kind: 6, batch: index as u32, ..Default::default() });
                    open_layer = Some((index, frame.vertices.len(), marker_depth));
                }
            }
            RenderCommand::CompositedEnd => {
                if let Some((index, first, depth)) = open_layer {
                    if depth == marker_depth {
                        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                        for v in &frame.vertices[first..] { x0 = x0.min(v.x); y0 = y0.min(v.y); x1 = x1.max(v.x); y1 = y1.max(v.y); }
                        frame.layers[index].bounds = if x0 <= x1 { [x0.floor() - 2.0, y0.floor() - 2.0, x1.ceil() + 2.0, y1.ceil() + 2.0] } else { [0.0; 4] };
                        frame.ops.push(Op { kind: 7, batch: index as u32, ..Default::default() });
                        open_layer = None;
                    }
                }
                marker_depth -= 1;
            }
            RenderCommand::Fill(f) => {
                let bm = frame.brush(&mut tess, &f.brush, f.transform, f.brush_transform);
                let (lin, off) = at_origin(f.transform);
                let key = Key::new().word(aa as u64).word(f.fill as u64).affine(lin).path(&f.shape).0;
                let geo = tess.get(key, || {
                    if aa { return aa_fill(&mut fill, &f.shape, lin, f.fill); }
                    let mut geo: Geo = VertexBuffers::new();
                    let rule = match f.fill { peniko::Fill::EvenOdd => FillRule::EvenOdd, _ => FillRule::NonZero };
                    let _ = fill.tessellate_path(&to_lyon(&f.shape, lin), &FillOptions::tolerance(TOLERANCE).with_fill_rule(rule),
                        &mut BuffersBuilder::new(&mut geo, |v: FillVertex| [v.position().x, v.position().y, 1.0]));
                    geo
                });
                frame.push(bm.kind, bm.texture, 0, painted_at(geo, &bm, 1.0, off));
            }
            RenderCommand::Stroke(s) => {
                let bm = frame.brush(&mut tess, &s.brush, s.transform, s.brush_transform);
                let c = s.transform.as_coeffs();
                let scale = (c[0] * c[3] - c[1] * c[2]).abs().sqrt() as f32;
                let st = &s.style;
                // A line thinner than a pixel is drawn one pixel wide at its share of coverage when edges are soft (it
                // would vanish between the fringes otherwise).
                let device = st.width as f32 * scale;
                let cov = if aa { device.clamp(0.0, 1.0) } else { 1.0 };
                let mut key = Key::new();
                let (lin, off) = at_origin(s.transform);
                key.word(10 + aa as u64).affine(lin).path(&s.shape).f(st.width).f(st.miter_limit)
                    .word(st.join as u64).word(st.start_cap as u64).word(st.end_cap as u64).f(st.dash_offset);
                for d in st.dash_pattern.iter() { key.f(*d); }
                let geo = tess.get(key.0, || {
                    if aa {
                        let mut style = st.clone();
                        if device < 1.0 { style.width = 1.0 / scale.max(1e-6) as f64; }
                        let outline = peniko::kurbo::stroke(s.shape.elements().iter().copied(), &style, &peniko::kurbo::StrokeOpts::default(), (TOLERANCE / scale.max(1e-6)) as f64);
                        return aa_fill(&mut fill, &outline, lin, peniko::Fill::NonZero);
                    }
                    let join = match st.join { peniko::kurbo::Join::Bevel => LineJoin::Bevel, peniko::kurbo::Join::Round => LineJoin::Round, _ => LineJoin::Miter };
                    let cap = |c: peniko::kurbo::Cap| match c { peniko::kurbo::Cap::Round => LineCap::Round, peniko::kurbo::Cap::Square => LineCap::Square, _ => LineCap::Butt };
                    let opts = StrokeOptions::tolerance(TOLERANCE)
                        .with_line_width(device)
                        .with_line_join(join)
                        .with_start_cap(cap(st.start_cap))
                        .with_end_cap(cap(st.end_cap))
                        .with_miter_limit((st.miter_limit as f32).max(1.0));
                    let mut geo: Geo = VertexBuffers::new();
                    let _ = stroke.tessellate_path(&to_lyon(&s.shape, lin), &opts,
                        &mut BuffersBuilder::new(&mut geo, |v: StrokeVertex| [v.position().x, v.position().y, 1.0]));
                    geo
                });
                frame.push(bm.kind, bm.texture, 0, painted_at(geo, &bm, cov, off));
            }
            RenderCommand::BoxShadow(b) => {
                // A blurred rounded rectangle (the same approximation as the CPU painter): a quad over the rectangle
                // grown by 2.5 σ; each pixel's position relative to the rectangle in uv, size, radius and σ in p.
                let rect = b.rect.abs();
                let k = 2.5 * b.std_dev;
                let outer = rect.inflate(k, k);
                let color = premul(b.brush, 1.0);
                let (w, h) = (rect.width() as f32, rect.height() as f32);
                let p = [w, h, b.radius as f32, b.std_dev as f32];
                let corners = [(outer.x0, outer.y0), (outer.x1, outer.y0), (outer.x1, outer.y1), (outer.x0, outer.y1)];
                let mut geo: VertexBuffers<Vertex, u32> = VertexBuffers::new();
                for (x, y) in corners {
                    let d = b.transform * peniko::kurbo::Point::new(x, y);
                    geo.vertices.push(Vertex { x: d.x as f32, y: d.y as f32, color, u: (x - rect.x0) as f32, v: (y - rect.y0) as f32, p, ..Default::default() });
                }
                geo.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
                frame.push(BOX_SHADOW, 0, 0, geo);
            }
            RenderCommand::GlyphRun(run) => {
                // Text: each glyph a quad from the atlas, at its device position (baseline on a whole pixel, x on a
                // quarter pixel, as Blitz placed them). Plain translations only; anything else waits for later steps.
                // A plain move or an even scale: glyphs from the atlas, rasterized at the size they show (so a scaled
                // label stays as crisp as an unscaled one). Rotated or skewed text, and glyphs too big for the atlas,
                // are drawn as outlines (triangles with soft edges).
                let c = run.transform.as_coeffs();
                // Colour glyphs (emoji) are pictures: drawn as images, whatever the transform.
                let device_ppem = run.font_size * (c[0] * c[3] - c[1] * c[2]).abs().sqrt() as f32;
                let mut colored = vec![false; run.glyphs.len()];
                for (i, g) in run.glyphs.iter().enumerate() {
                    let Some((data, local)) = glyphs.color_glyph(run, g.id, device_ppem) else { continue };
                    colored[i] = true;
                    let place = run.transform * Affine::translate((g.x as f64, g.y as f64)) * local;
                    let (w, h) = (data.width as f64, data.height as f64);
                    let mut brush = peniko::ImageBrush::new(data);
                    brush.sampler.alpha = run.brush_alpha;
                    let bm = frame.brush(&mut tess, &Paint::Image(brush), place, None);
                    let quad = peniko::kurbo::Shape::to_path(&peniko::kurbo::Rect::new(0.0, 0.0, w, h), 0.1);
                    let (lin, off) = at_origin(place);
                    let key = Key::new().word(40).affine(lin).f(w).f(h).0;
                    let geo = tess.get(key, || aa_fill(&mut fill, &quad, lin, peniko::Fill::NonZero));
                    frame.push(bm.kind, bm.texture, 0, painted_at(geo, &bm, 1.0, off));
                }
                let even = c[1].abs() < 1e-6 && c[2].abs() < 1e-6 && c[0] > 0.0 && (c[0] - c[3]).abs() < 1e-6;
                if !even || run.font_size * c[0] as f32 > GLYPH_OUTLINE {
                    let bm = frame.brush(&mut tess, &run.brush, run.transform, None);
                    for (i, g) in run.glyphs.iter().enumerate() {
                        if colored[i] { continue; }
                        let place = run.transform * Affine::translate((g.x as f64, g.y as f64)) * run.glyph_transform.unwrap_or(Affine::IDENTITY);
                        // Emboldening (user units) as device px: stems grow on both sides, glyphs rise by the
                        // vertical amount so the baseline stays put.
                        let pc = place.as_coeffs();
                        let scale = (pc[0] * pc[3] - pc[1] * pc[2]).abs().sqrt();
                        let grow = (run.embolden.x * scale) as f32;
                        let (lin, off) = at_origin(place * Affine::translate((0.0, -run.embolden.y)));
                        let key = Key::new().word(30).word(run.font_data.data.id()).word(run.font_data.index as u64).word(g.id as u64)
                            .f(run.font_size as f64).affine(lin).f(run.embolden.x).f(run.embolden.y).0;
                        let geo = tess.get(key, || match glyph_outline(run, g.id) {
                            Some(path) => aa_fill_grown(&mut fill, &path, lin, peniko::Fill::NonZero, grow),
                            None => VertexBuffers::new(),
                        });
                        frame.push(bm.kind, bm.texture, 0, painted_at(geo, &bm, run.brush_alpha, off));
                    }
                    continue;
                }
                let k = c[0];
                let color = match &run.brush {
                    Paint::Solid(col) => premul(*col, run.brush_alpha),
                    Paint::Gradient(g) => g.stops.first().map(|s| premul(s.color.to_alpha_color(), run.brush_alpha)).unwrap_or(0),
                    _ => continue,
                };
                let mut geo: VertexBuffers<Vertex, u32> = VertexBuffers::new();
                for (i, g) in run.glyphs.iter().enumerate() {
                    if colored[i] { continue; }
                    let gx = c[4] + g.x as f64 * k;
                    let gy = c[5] + g.y as f64 * k;
                    let ix = gx.floor();
                    let sub = (((gx - ix) * 4.0).round() as u8).min(3);
                    let found = match glyphs.get(run, run.font_size * k as f32, g.id, sub) {
                        Ok(e) => e,
                        // A full atlas starts over; quads made earlier in this frame point at the old glyphs, so the
                        // frame is built again from the top by the caller.
                        Err(()) => { glyphs.clear(); glyphs.overflowed = true; glyphs.get(run, run.font_size * k as f32, g.id, sub).ok().flatten() }
                    };
                    let Some(e) = found else { continue };
                    let x0 = (ix + e.left as f64) as f32;
                    let y0 = (gy.round() + e.top as f64) as f32;
                    let (x1, y1) = (x0 + e.w as f32, y0 + e.h as f32);
                    let a = ATLAS as f32;
                    let (u0, v0, u1, v1) = (e.x as f32 / a, e.y as f32 / a, (e.x + e.w) as f32 / a, (e.y + e.h) as f32 / a);
                    let base = geo.vertices.len() as u32;
                    geo.vertices.push(Vertex { x: x0, y: y0, color, u: u0, v: v0, p: [0.0; 4], ..Default::default() });
                    geo.vertices.push(Vertex { x: x1, y: y0, color, u: u1, v: v0, p: [0.0; 4], ..Default::default() });
                    geo.vertices.push(Vertex { x: x1, y: y1, color, u: u1, v: v1, p: [0.0; 4], ..Default::default() });
                    geo.vertices.push(Vertex { x: x0, y: y1, color, u: u0, v: v1, p: [0.0; 4], ..Default::default() });
                    geo.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
                }
                frame.push(GLYPH, 0, 0, geo);
            }
        }
    }
    frame.pool.end(&frame.vertices);
    frame.vertex_spans = frame.pool.written.clone();
    // What changed on the page: where the new draws are and where the freed ones were, unless the structure changed.
    let mut sig = Key::new();
    for op in &frame.ops {
        sig.word(op.kind as u64).f(op.alpha as f64).word(op.compose as u64).word(op.mix as u64).word(op.filter_count as u64);
        if op.kind != 0 { sig.word(op.batch as u64); }
    }
    for l in &frame.layers { sig.word(l.id); }
    for sc in &frame.scrolls { sig.word(sc.id); }
    let structure = sig.0;
    // Each changed draw's box, new and freed, joined into at most four.
    let mut boxes: Vec<[f32; 4]> = frame.pool.freed.clone();
    boxes.extend_from_slice(&frame.pool.placed);
    frame.dirty = if structure != tess.structure { vec![[0.0, 0.0, f32::MAX, f32::MAX]] } else { join_boxes(boxes, 4) };
    tess.structure = structure;
    let (w0, w1) = (frame.vertex_spans.first().map(|s| s.0).unwrap_or(0), frame.vertex_spans.last().map(|s| s.1).unwrap_or(0));
    frame.changed[0] = w0;
    frame.changed[1] = w1;
    tess.pool = std::mem::take(&mut frame.pool);
    frame.clip_table.end();
    tess.ramp_table.end();
    frame.clips = frame.clip_table.rows.clone();
    frame.ramps = tess.ramp_table.rows.clone();
    tess.clip_table = std::mem::take(&mut frame.clip_table);
    tess.end();
    glyphs.tess = tess;
    frame
}
