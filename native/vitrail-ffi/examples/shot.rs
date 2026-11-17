//! Paints a page with the library's CPU painter (scripts and animations run for `seconds` first) and writes a PNG.
//! Used for the pictures in `media/`. Usage: shot <page.html> <out.png> <css width> <css height> [scale] [seconds]
use std::ffi::CString;
use vitrail::*;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let page = std::fs::canonicalize(&a[1]).expect("page");
    let (w, h): (u32, u32) = (a[3].parse().unwrap(), a[4].parse().unwrap());
    let scale: f32 = a.get(5).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let seconds: f64 = a.get(6).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let (pw, ph) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
    let html = CString::new(std::fs::read_to_string(&page).unwrap()).unwrap();
    let url = CString::new(format!("file://{}", page.display())).unwrap();
    let mut px = vec![0u8; (pw * ph * 4) as usize];
    unsafe {
        let ui = vitrail_create(html.as_ptr(), url.as_ptr(), pw, ph, scale);
        let steps = (seconds * 20.0).ceil() as i32;
        for i in 0..=steps { vitrail_tick(ui, i as f64 / 20.0); }
        vitrail_render(ui, px.as_mut_ptr(), px.len());
        vitrail_destroy(ui);
    }
    // The painter gives premultiplied alpha; PNG wants it straight.
    for p in px.chunks_mut(4) {
        let al = p[3] as u32;
        if al > 0 && al < 255 { for c in &mut p[..3] { *c = ((*c as u32 * 255 + al / 2) / al).min(255) as u8; } }
    }
    let file = std::fs::File::create(&a[2]).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), pw, ph);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&px).unwrap();
}
