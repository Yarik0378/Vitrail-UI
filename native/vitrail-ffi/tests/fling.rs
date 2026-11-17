//! A fast fling to the bottom of a long list, painted on the worker as the game does it: once the wheel stops, the
//! last frame must hold the content at the final offset (no blank part left on screen), and the frames on the way
//! showing past the held content are counted.
//! Run: cargo test --release --test fling -- --nocapture
use std::ffi::CString;
use vitrail::*;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

unsafe fn held(ui: *mut Vitrail) -> (f32, f32) {
    unsafe {
        let mut info = [0u8; 40 * 8];
        let n = vitrail_gpu_scrolls(ui, info.as_mut_ptr(), info.len());
        let mut now = [0f32; 16];
        vitrail_scroll_states(ui, now.as_mut_ptr(), 8);
        assert!(n >= 1);
        let painted = f32::from_ne_bytes(info[20..24].try_into().unwrap());
        (painted, now[1])
    }
}

/// Where the list really is (the document's offset, device px).
unsafe fn real(ui: *mut Vitrail) -> f32 {
    unsafe {
        let mut e = [0f32; 64];
        let n = vitrail_scroll_extents(ui, e.as_mut_ptr(), 8);
        assert!(n >= 1);
        e[1]
    }
}

#[test]
fn fling_to_the_bottom_ends_painted() {
    unsafe {
        let page = format!("{}/../../tests/pages/30-inventory.html", env!("CARGO_MANIFEST_DIR"));
        let ui = vitrail_create(s(&std::fs::read_to_string(&page).unwrap()).as_ptr(), s(&format!("file://{page}")).as_ptr(), 1920, 1080, 1.0);
        vitrail_set_gpu(ui, 2);
        let (mut rect, mut regions) = ([0u32; 4], 0u32);
        let mut frame = |dy: f32| -> bool {
            vitrail_paint_poll(ui, std::ptr::null_mut(), 0, rect.as_mut_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), 0, &mut regions);
            if dy != 0.0 { vitrail_scroll(ui, 900.0, 500.0, 0.0, dy); }
            let started = vitrail_needs_paint(ui) != 0 && vitrail_paint_start(ui) != 0;
            std::thread::sleep(std::time::Duration::from_micros(2000));
            started
        };
        // Until the first frame is in (slower while other tests run beside).
        for _ in 0..3000 { frame(0.0); if vitrail_gpu_scrolls(ui, std::ptr::null_mut(), 0) > 0 && vitrail_needs_paint(ui) == 0 { break; } }
        let margin = 600.0;
        let (mut blank, mut waited) = (0, 0);
        for _ in 0..40 {
            frame(1000.0);
            let (p, n) = held(ui);
            if (n - p).abs() > margin { blank += 1; }
            if (real(ui) - n).abs() > 0.5 { waited += 1; }
        }
        for _ in 0..100 { frame(0.0); }
        let (p, n) = held(ui);
        let r = real(ui);
        println!("fling: {blank}/40 frames showed past the held content, {waited} waited at its edge; at rest painted at {p}, shown at {n}, the list at {r}");
        assert_eq!(blank, 0, "frames showed blank space past the held content");
        assert!((n - r).abs() < 0.5 && (n - p).abs() <= margin * 0.5, "at rest painted at {p}, shown at {n}, the list at {r}");
        vitrail_destroy(ui);
    }
}
