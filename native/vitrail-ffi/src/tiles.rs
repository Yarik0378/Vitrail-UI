//! Tiled repaint (as browsers do): the page is cut into 256 px tiles; each tile remembers which draw commands touch it,
//! and a repaint rasterizes only the tiles whose commands differ from the last paint. A hover or a number costs the size
//! of what changed, not the size of the page.

use anyrender::recording::RenderCommand;
use anyrender::{ImageRenderer, PaintScene, Scene, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use peniko::kurbo::{Affine, Rect, Shape};

pub const TILE: u32 = 256;

/// What the last paint left: its commands and, per tile, the commands that touched it.
pub struct TileState {
    /// Painters kept from paint to paint, by size (making one starts its threads and allocates its buffers).
    painters: std::collections::HashMap<(u32, u32), VelloCpuImageRenderer>,
    scene: Scene,
    tiles: Vec<Vec<usize>>,
    w: u32,
    h: u32,
}

/// The device-space box a command can touch (with room for antialiasing), or None for layer structure.
fn bounds(cmd: &RenderCommand) -> Option<Rect> {
    const PAD: f64 = 2.0;
    let r = match cmd {
        RenderCommand::PushLayer(_) | RenderCommand::PushClipLayer(_) | RenderCommand::PopLayer
        | RenderCommand::CompositedBegin(_) | RenderCommand::CompositedEnd
        | RenderCommand::ScrollBegin(_) | RenderCommand::ScrollEnd | RenderCommand::Group(_) => return None,
        RenderCommand::Fill(f) => f.transform.transform_rect_bbox(f.shape.bounding_box()),
        RenderCommand::Stroke(s) => {
            let w = s.style.width;
            s.transform.transform_rect_bbox(s.shape.bounding_box().inflate(w, w))
        }
        RenderCommand::GlyphRun(g) => {
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for gl in &g.glyphs {
                x0 = x0.min(gl.x as f64);
                y0 = y0.min(gl.y as f64);
                x1 = x1.max(gl.x as f64);
                y1 = y1.max(gl.y as f64);
            }
            if x0 > x1 { return Some(Rect::ZERO); }
            let fs = g.font_size as f64 * 1.5;
            let local = Rect::new(x0 - fs, y0 - fs, x1 + fs, y1 + fs);
            let local = match g.glyph_transform { Some(t) => t.transform_rect_bbox(local).union(local), None => local };
            g.transform.transform_rect_bbox(local)
        }
        RenderCommand::BoxShadow(b) => {
            let e = b.std_dev * 3.0 + b.radius;
            b.transform.transform_rect_bbox(b.rect.inflate(e, e))
        }
    };
    Some(r.inflate(PAD, PAD))
}

/// Paints `page` into `front` (w×h RGBA8, reused from the last paint), rasterizing only the tiles that changed.
/// Returns the new state and the box of the pixels that actually differ.
pub fn paint(page: Scene, prev: Option<TileState>, front: &mut Vec<u8>, w: u32, h: u32) -> (TileState, Option<[u32; 4]>) {
    let (cols, rows) = (w.div_ceil(TILE), h.div_ceil(TILE));
    let boxes: Vec<Option<Rect>> = page.commands.iter().map(bounds).collect();
    // A filter layer reads pixels around it, which a tile on its own doesn't have: such pages paint whole.
    let filtered = page.commands.iter().any(|c| matches!(c, RenderCommand::PushLayer(l) if l.filter.is_some()));
    let mut tiles = vec![Vec::new(); (cols * rows) as usize];
    for (i, b) in boxes.iter().enumerate() {
        match b {
            None => tiles.iter_mut().for_each(|t| t.push(i)),
            Some(r) => {
                if r.width() <= 0.0 || r.height() <= 0.0 { continue; }
                let c0 = (r.x0.max(0.0) / TILE as f64) as u32;
                let r0 = (r.y0.max(0.0) / TILE as f64) as u32;
                let c1 = ((r.x1.max(0.0) / TILE as f64) as u32).min(cols.saturating_sub(1));
                let r1 = ((r.y1.max(0.0) / TILE as f64) as u32).min(rows.saturating_sub(1));
                if r.x0 >= w as f64 || r.y0 >= h as f64 { continue; }
                for ty in r0..=r1 {
                    for tx in c0..=c1 {
                        tiles[(ty * cols + tx) as usize].push(i);
                    }
                }
            }
        }
    }
    let fresh = front.len() != (w * h * 4) as usize || filtered || prev.as_ref().map(|p| p.w != w || p.h != h).unwrap_or(true);
    if front.len() != (w * h * 4) as usize { *front = vec![0u8; (w * h * 4) as usize]; }
    let mut prev = prev;
    let mut painters = prev.as_mut().map(|p| std::mem::take(&mut p.painters)).unwrap_or_default();
    let mut tile_buf: Vec<u8> = Vec::new();
    let mut dirty: Option<[u32; 4]> = None;
    let mut grow = |x0: u32, y0: u32, x1: u32, y1: u32| {
        dirty = Some(match dirty {
            None => [x0, y0, x1, y1],
            Some([a, b, c, d]) => [a.min(x0), b.min(y0), c.max(x1), d.max(y1)],
        });
    };
    if fresh && filtered {
        // The whole page in one go.
        let buf = render_to_buffer::<VelloCpuImageRenderer, _>(|s| s.append_scene(page.clone(), Affine::IDENTITY), w, h);
        if let Some([x, y, cw, ch]) = crate::diff_rect(front, &buf, w as usize, h as usize) { grow(x, y, x + cw, y + ch); }
        *front = buf;
    } else {
        // Which tiles changed.
        let mut changed = vec![false; (cols * rows) as usize];
        for idx in 0..(cols * rows) as usize {
            let same = !fresh && prev.as_ref().map(|p| {
                let (a, b) = (&tiles[idx], &p.tiles[idx]);
                a.len() == b.len() && a.iter().zip(b).all(|(&i, &j)| page.commands[i] == p.scene.commands[j])
            }).unwrap_or(false);
            changed[idx] = !same;
        }
        // Changed tiles merged into rectangles (runs along a row, then equal runs down the rows), each painted at once:
        // a painter call costs about as much as a small tile, so fewer bigger calls win.
        let mut rects: Vec<[u32; 4]> = Vec::new(); // tile coords: x0, y0, x1, y1 (exclusive)
        for ty in 0..rows {
            let mut tx = 0;
            while tx < cols {
                if !changed[(ty * cols + tx) as usize] { tx += 1; continue; }
                let start = tx;
                while tx < cols && changed[(ty * cols + tx) as usize] { tx += 1; }
                if let Some(r) = rects.iter_mut().find(|r| r[0] == start && r[2] == tx && r[3] == ty) { r[3] = ty + 1; }
                else { rects.push([start, ty, tx, ty + 1]); }
            }
        }
        for [c0, r0, c1, r1] in rects {
            let (x0, y0) = (c0 * TILE, r0 * TILE);
            let (rw, rh) = ((c1 * TILE).min(w) - x0, (r1 * TILE).min(h) - y0);
            let (pw, ph) = ((c1 - c0) * TILE, (r1 - r0) * TILE);
            // Every command that touches any tile of the rectangle, once, in order.
            let mut picked = vec![false; page.commands.len()];
            for ty in r0..r1 { for tx in c0..c1 { for &i in &tiles[(ty * cols + tx) as usize] { picked[i] = true; } } }
            let mut sub = Scene::new();
            sub.commands = page.commands.iter().zip(&picked).filter(|(_, p)| **p).map(|(c, _)| c.clone()).collect();
            let painter = painters.entry((pw, ph)).or_insert_with(|| VelloCpuImageRenderer::new(pw, ph));
            if tile_buf.len() < (pw * ph * 4) as usize { tile_buf.resize((pw * ph * 4) as usize, 0); }
            painter.reset();
            painter.render(|s| s.append_scene(sub, Affine::translate((-(x0 as f64), -(y0 as f64)))), &mut tile_buf[..(pw * ph * 4) as usize]);
            // Copy in, noting the rows and columns that really changed.
            let (mut cx0, mut cy0, mut cx1, mut cy1) = (u32::MAX, u32::MAX, 0, 0);
            for row in 0..rh {
                let dst = (((y0 + row) * w + x0) * 4) as usize;
                let src = (row * pw * 4) as usize;
                let (old, new) = (&mut front[dst..dst + (rw * 4) as usize], &tile_buf[src..src + (rw * 4) as usize]);
                if old != new {
                    let first = old.chunks_exact(4).zip(new.chunks_exact(4)).position(|(p, q)| p != q).unwrap_or(0) as u32;
                    let last = rw - 1 - old.chunks_exact(4).rev().zip(new.chunks_exact(4).rev()).position(|(p, q)| p != q).unwrap_or(0) as u32;
                    cx0 = cx0.min(first); cx1 = cx1.max(last + 1); cy0 = cy0.min(row); cy1 = row + 1;
                    old.copy_from_slice(new);
                }
            }
            if cx0 != u32::MAX { grow(x0 + cx0, y0 + cy0, x0 + cx1, y0 + cy1); }
        }
    }
    // A fresh page goes up whole: the texture on the other side starts with nothing in it.
    let rect = if fresh { Some([0, 0, w, h]) } else { dirty.map(|[x0, y0, x1, y1]| [x0, y0, x1 - x0, y1 - y0]) };
    if painters.len() > 16 { painters.clear(); }
    (TileState { painters, scene: page, tiles, w, h }, rect)
}
