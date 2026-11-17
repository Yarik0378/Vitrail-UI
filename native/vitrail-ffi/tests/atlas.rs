//! Dumps the glyph atlas after building a page's GPU frame (cargo test --release -p vitrail --test atlas -- <page>).
use std::ffi::CString;
use vitrail::*;

#[test]
fn dump_atlas() {
    let page = std::env::var("PAGE").unwrap_or_else(|_| format!("{}/../../tests/pages/07-bar.html", env!("CARGO_MANIFEST_DIR")));
    let html = std::fs::read_to_string(&page).unwrap();
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new(format!("file://{page}")).unwrap().as_ptr(), 1422, 118, 1.0);
        let mut counts = [0u32; 7];
        vitrail_gpu_build(ui, counts.as_mut_ptr());
        let mut atlas = vec![0u8; 2048 * 2048];
        vitrail_gpu_atlas(ui, atlas.as_mut_ptr(), atlas.len());
        let (w, h) = (512u32, 64u32);
        let mut out = vec![0u8; (w * h) as usize];
        for y in 0..h { for x in 0..w { out[(y * w + x) as usize] = atlas[(y * 2048 + x) as usize]; } }
        let mut enc = png::Encoder::new(std::fs::File::create(std::env::temp_dir().join("vitrail-atlas.png")).unwrap(), w, h);
        enc.set_color(png::ColorType::Grayscale); enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&out).unwrap();
        vitrail_destroy(ui);
    }
}
