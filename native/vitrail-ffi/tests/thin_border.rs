//! Thin borders at fractional device scales, set before or after the first paint. A turn panel at HUD scale
//! 2.529274 lost the 1 px underline of a link: Stylo snaps a 1 px border to 2 device px (0.79 CSS px)
//! and Blitz then rounded layout to whole CSS px, which rounded it to 0. Layout now rounds to device pixels.
use std::ffi::CString;
use vitrail::*;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

unsafe fn paint(ui: *mut Vitrail) {
    let (mut rect, mut regions) = ([0u32; 4], 0u32);
    unsafe {
        vitrail_paint_start(ui);
        while vitrail_paint_poll(ui, std::ptr::null_mut(), 0, rect.as_mut_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), 0, &mut regions) == 0 { std::thread::yield_now(); }
    }
}

/// Triangles drawn in the band of `#skip`'s bottom border (its last `border` device px), across its width.
unsafe fn underline_triangles(ui: *mut Vitrail, border: f32) -> usize {
    unsafe {
        let mut r = [0f32; 4];
        assert_eq!(vitrail_rect(ui, s("#skip").as_ptr(), r.as_mut_ptr()), 1, "no #skip");
        let mut c = [0u32; 7];
        vitrail_gpu_counts(ui, c.as_mut_ptr());
        let (nv, ni, nb) = (c[0] as usize, c[1] as usize, c[2] as usize);
        let mut v = vec![0u8; nv * 60];
        let mut idx = vec![0u32; ni];
        let mut b = vec![0u8; nb * 256];
        assert_eq!(vitrail_gpu_copy(ui, v.as_mut_ptr(), v.len(), idx.as_mut_ptr(), idx.len(), b.as_mut_ptr(), b.len()), 1);
        let at = |i: u32| {
            let o = i as usize * 60;
            (f32::from_le_bytes(v[o..o + 4].try_into().unwrap()), f32::from_le_bytes(v[o + 4..o + 8].try_into().unwrap()))
        };
        let (x0, x1, y1) = (r[0] - 2.0, r[0] + r[2] + 2.0, r[1] + r[3]);
        let wide = if std::env::var_os("WIDE").is_some() { 6.0 } else { 1.5 }; let (top, bottom) = (y1 - border - wide, y1 + wide);
        let found: Vec<&[u32]> = idx.chunks(3).filter(|t| t.iter().all(|&i| { let (x, y) = at(i); x >= x0 && x <= x1 && y >= top && y <= bottom })).collect();
        if std::env::var_os("DUMP").is_some() {
            let u = |o: usize| u32::from_le_bytes(v[o..o + 4].try_into().unwrap());
            for t in &found {
                let o = t[0] as usize * 60;
                let (meta, meta2) = (u(o + 36), u(o + 40));
                println!("  tri {:?} color {:08x} kind {} clip1 {} clip2 {} clip3 {}", t.iter().map(|&i| at(i)).collect::<Vec<_>>(), u(o + 8), meta & 255, (meta >> 8) & 0xffff, meta2 & 0xffff, meta2 >> 16);
            }
            let nc = vitrail_gpu_clips(ui, std::ptr::null_mut(), 0) as usize;
            let mut cl = vec![0f32; nc * 16];
            vitrail_gpu_clips(ui, cl.as_mut_ptr(), cl.len());
            for k in 0..nc { println!("  clip {}: {:?}", k + 1, &cl[k * 16..k * 16 + 8]); }
        }
        found.len()
    }
}

#[test]
fn thin_border_survives_fractional_scales() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/pages/game-hud");
    let html = std::fs::read_to_string(dir.join("turn.html")).unwrap();
    let base = s(&format!("file://{}/turn.html", dir.display()));
    // The game's scale first (4K editor view), then others; SCALE checks one.
    let scales: Vec<f32> = match std::env::var("SCALE").ok().and_then(|v| v.parse().ok()) { Some(v) => vec![v], None => vec![2.529274, 1.0, 1.139, 1.5, 2.0, 2.15, 2.53, 3.0] };
    for scale in scales { unsafe { check(&html, &base, scale) } }
}

unsafe fn check(html: &str, base: &CString, scale: f32) {
    let (w, h) = ((288.0 * scale).ceil() as u32, (380.0 * scale).ceil() as u32);
    unsafe {
        // From the start: the panel asks, so «Все одно завершити» shows with its underline.
        let early = vitrail_create(s(&html.replace("class=\"panel\"", "class=\"panel ask warm\"")).as_ptr(), base.as_ptr(), w, h, scale);
        vitrail_set_gpu(early, 2);
        paint(early);
        let want = underline_triangles(early, scale);
        // Later: the same classes set after a first paint, as the game does.
        let late = vitrail_create(s(&html).as_ptr(), base.as_ptr(), w, h, scale);
        vitrail_set_gpu(late, 2);
        paint(late);
        // What the game's TurnPanel.Show sets (ONLY=n keeps just step n, to find the culprit).
        let only: Option<usize> = std::env::var("ONLY").ok().and_then(|v| v.parse().ok());
        let steps: Vec<Box<dyn Fn()>> = vec![
            Box::new(|| { vitrail_set_text(late, s("#turn").as_ptr(), s("Хід 1").as_ptr()); }),
            Box::new(|| { vitrail_set_text(late, s("#season-text").as_ptr(), s("Весна · рік 1").as_ptr()); }),
            Box::new(|| { vitrail_set_text(late, s("#caption").as_ptr(), s("2 загони чекають наказу").as_ptr()); }),
            Box::new(|| { vitrail_set_class(late, s("#panel").as_ptr(), s("warn-off").as_ptr(), 0); }),
            Box::new(|| { vitrail_set_style(late, s("#seasons").as_ptr(), s("background").as_ptr(), s("conic-gradient(rgba(140, 196, 106, 1) 0% 25.6%, transparent 25.6% 27%, rgba(233, 198, 106, .42) 27% 52.6%, transparent 52.6% 54%, rgba(224, 154, 90, .42) 54% 79.6%, transparent 79.6% 81%, rgba(169, 198, 220, .42) 81% 98.6%, transparent 98.6% 100%)").as_ptr()); }),
            Box::new(|| { vitrail_set_style(late, s("#mark").as_ptr(), s("transform").as_ptr(), s("rotate(6.75deg)").as_ptr()); }),
            Box::new(|| { vitrail_set_style(late, s("#warn").as_ptr(), s("transform").as_ptr(), s("rotate(320deg)").as_ptr()); }),
            Box::new(|| { vitrail_set_attr(late, s("#icon").as_ptr(), s("src").as_ptr(), s("icons/units/warrior.svg").as_ptr()); }),
            Box::new(|| { vitrail_set_attr(late, s("#icon").as_ptr(), s("class").as_ptr(), s("icon unit").as_ptr()); }),
            Box::new(|| { vitrail_set_text(late, s("#season-tip").as_ptr(), s("Пори року: що дає весна, прогноз і зима").as_ptr()); }),
            Box::new(|| { vitrail_set_text(late, s("#dial-tip").as_ptr(), s("Хід 1 · весна, рік 1\nДо наступної пори: 4 ходи\nСпершу: загін чекає наказу (Enter)").as_ptr()); }),
        ];
        for (k, step) in steps.iter().enumerate() { if only.is_none_or(|o| o == k) { step(); } }
        vitrail_set_class(late, s("#panel").as_ptr(), s("ask").as_ptr(), 1);
        vitrail_set_class(late, s("#panel").as_ptr(), s("warm").as_ptr(), 1);
        paint(late);
        // The game's clock: the breathing ring's animation makes it a composited layer.
        if std::env::var_os("NOTICK").is_none() {
            for t in [0.1f64, 0.5, 1.0] { vitrail_tick(late, t); if vitrail_needs_paint(late) != 0 { paint(late); } }
        }
        let got = underline_triangles(late, scale);
        println!("scale {scale}: underline triangles from the start {want}, set later {got}");
        assert!(want > 0, "scale {scale}: no underline on the page that had it from the start");
        assert_eq!(got, want);
        vitrail_destroy(early);
        vitrail_destroy(late);
    }
}
