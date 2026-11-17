//! A page renders the same whatever was rendered before it in the process (no state leaks between documents).
use std::ffi::CString;
use vitrail::*;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

unsafe fn shot(page: &str, w: u32, h: u32) -> Vec<u8> {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests");
    let file = tests.join(page);
    let html = std::fs::read_to_string(&file).unwrap();
    let mut px = vec![0u8; (w * h * 4) as usize];
    unsafe {
        let ui = vitrail_create(s(&html).as_ptr(), s(&format!("file://{}", file.canonicalize().unwrap().display())).as_ptr(), w, h, 1.0);
        for i in 0..=20 { vitrail_tick(ui, i as f64 * 0.1); }
        vitrail_render(ui, px.as_mut_ptr(), px.len());
        vitrail_destroy(ui);
    }
    px
}

#[test]
fn pages_do_not_leak_into_the_next() {
    unsafe {
        let alone = shot("pages/10-filter.html", 700, 120);
        for before in ["pages/01-box.html", "pages/03-shadows.html", "pages/05-svg.html", "pages/06-topbar.html", "pages/07-bar.html"] {
            let _ = shot(before, 1422, 130);
            let after = shot("pages/10-filter.html", 700, 120);
            let diff: f64 = alone.iter().zip(&after).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / alone.len() as f64;
            println!("after {before}: {diff:.2}");
            assert!(diff < 0.5, "10-filter changed after {before}: {diff:.2}");
        }
    }
}
