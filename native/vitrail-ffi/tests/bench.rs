//! The main thread's cost per change on the game's real top bar at 4K (run: cargo test --release -p vitrail --test bench -- --nocapture).
use std::ffi::CString;
use std::time::Instant;
use vitrail::*;

#[test]
fn topbar_main_thread_cost() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/pages/game-hud");
    let html = std::fs::read_to_string(dir.join("topbar.html")).unwrap().replace("<body>", "<body class=\"founded\">");
    let base = format!("file://{}/topbar.html", dir.display());
    let (w, h, s) = (3840u32, 323u32, 2.15f32);
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new(base).unwrap().as_ptr(), w, h, s);
        let mut out = vec![0u8; (w * h * 4) as usize];
        let mut mask = out.clone();
        let (mut rect, mut params, mut regions) = ([0u32; 4], [0f32; 168], 0u32);
        let mut finish = |ui| loop {
            if vitrail_paint_poll(ui, out.as_mut_ptr(), out.len(), rect.as_mut_ptr(), mask.as_mut_ptr(), mask.len(), params.as_mut_ptr(), 8, &mut regions) != 0 { break; }
            std::thread::sleep(std::time::Duration::from_micros(200));
        };
        vitrail_paint_start(ui); finish(ui);
        let spots = [(3700.0, 60.0), (3500.0, 60.0), (100.0, 80.0), (900.0, 50.0), (2000.0, 40.0)];
        let (mut hover, mut start, mut worker) = (0.0, 0.0, 0.0);
        let n = 40;
        for i in 0..n {
            let (x, y) = spots[i % spots.len()];
            let t0 = Instant::now();
            vitrail_pointer_move(ui, x, y);
            let t1 = Instant::now();
            vitrail_paint_start(ui);
            let t2 = Instant::now();
            finish(ui);
            let t3 = Instant::now();
            hover += (t1 - t0).as_secs_f64(); start += (t2 - t1).as_secs_f64(); worker += (t3 - t2).as_secs_f64();
        }
        let ms = |v: f64| v / n as f64 * 1000.0;
        println!("per hover: pointer_move {:.2} ms, paint_start {:.2} ms (main thread); worker {:.2} ms", ms(hover), ms(start), ms(worker));
        vitrail_destroy(ui);
    }
}

#[test]
fn fullscreen_window_cost() {
    // A 4K window full of cards (like a tech tree): a hover must cost the card, not the screen.
    let mut html = String::from(r#"<!doctype html><html><head><style>
html,body{margin:0;background:#16241f;font:600 15px sans-serif;color:#eee}
.grid{display:grid;grid-template-columns:repeat(12,1fr);gap:14px;padding:20px}
.card{height:120px;border-radius:12px;background:linear-gradient(#2a4a43,#0c1b18);box-shadow:0 4px 10px rgba(0,0,0,.5),inset 0 0 0 1px #5b7d74;padding:10px}
.card:hover{background:linear-gradient(#3a6a5f,#14302a)}
</style></head><body><div class="grid">"#);
    for i in 0..96 { html.push_str(&format!("<div class=\"card\">Знання {i}<br>Відкриває будівлю</div>")); }
    html.push_str("</div></body></html>");
    let (w, h, s) = (3840u32, 2160u32, 2.0f32);
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), std::ptr::null(), w, h, s);
        let mut out = vec![0u8; (w * h * 4) as usize];
        let mut mask = out.clone();
        let (mut rect, mut params, mut regions) = ([0u32; 4], [0f32; 168], 0u32);
        let mut finish = |ui| loop {
            if vitrail_paint_poll(ui, out.as_mut_ptr(), out.len(), rect.as_mut_ptr(), mask.as_mut_ptr(), mask.len(), params.as_mut_ptr(), 8, &mut regions) != 0 { break; }
            std::thread::sleep(std::time::Duration::from_micros(200));
        };
        let t = Instant::now();
        vitrail_paint_start(ui); finish(ui);
        println!("4K window first paint {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        let (mut start, mut worker) = (0.0, 0.0);
        let n = 30;
        for i in 0..n {
            let (x, y) = (200.0 + (i % 6) as f32 * 600.0, 200.0 + (i % 4) as f32 * 500.0);
            vitrail_pointer_move(ui, x, y);
            let t1 = Instant::now();
            vitrail_paint_start(ui);
            let t2 = Instant::now();
            finish(ui);
            let t3 = Instant::now();
            start += (t2 - t1).as_secs_f64(); worker += (t3 - t2).as_secs_f64();
        }
        println!("4K window per hover: main {:.2} ms, worker {:.2} ms, last change {}x{}", start / n as f64 * 1000.0, worker / n as f64 * 1000.0, rect[2], rect[3]);
        vitrail_destroy(ui);
    }
}

#[test]
fn gpu_build_cost() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/pages/game-hud");
    let html = std::fs::read_to_string(dir.join("topbar.html")).unwrap().replace("<body>", "<body class=\"founded\">");
    let base = format!("file://{}/topbar.html", dir.display());
    unsafe {
        for mode in [1, 2] {
        let ui = vitrail_create(CString::new(html.clone()).unwrap().as_ptr(), CString::new(base.clone()).unwrap().as_ptr(), 3840, 323, 2.15);
        vitrail_set_gpu(ui, mode);
        let mut c = [0u32; 7];
        vitrail_gpu_build(ui, c.as_mut_ptr());
        let spots = [(3700.0, 60.0), (3500.0, 60.0), (100.0, 80.0), (900.0, 50.0)];
        let n = 40;
        let t = Instant::now();
        for i in 0..n { let (x, y) = spots[i % 4]; vitrail_pointer_move(ui, x, y); vitrail_gpu_build(ui, c.as_mut_ptr()); }
        println!("gpu build per hover (top bar 4K, {}): {:.2} ms, {} vertices, {} ops", if mode == 2 { "edge aa" } else { "msaa" }, t.elapsed().as_secs_f64() * 1000.0 / n as f64, c[0], c[5]);
        vitrail_destroy(ui);
        }
    }
}

#[test]
fn gpu_animation_cost() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/pages/game-hud");
    let html = std::fs::read_to_string(dir.join("topbar.html")).unwrap().replace("<body>", "<body class=\"founded\">");
    let base = format!("file://{}/topbar.html", dir.display());
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new(base).unwrap().as_ptr(), 3840, 323, 2.15);
        vitrail_set_gpu(ui, 2);
        let mut c = [0u32; 7];
        vitrail_tick(ui, 0.0);
        vitrail_gpu_build(ui, c.as_mut_ptr());
        let sel = CString::new(".btns").unwrap();
        vitrail_set_style(ui, sel.as_ptr(), CString::new("transition").unwrap().as_ptr(), CString::new("transform 20s linear").unwrap().as_ptr());
        vitrail_set_style(ui, sel.as_ptr(), CString::new("transform").unwrap().as_ptr(), CString::new("translateX(-600px)").unwrap().as_ptr());
        let n = 60;
        let t = Instant::now();
        for i in 0..n { vitrail_tick(ui, 0.1 + i as f64 / 60.0); vitrail_gpu_build(ui, c.as_mut_ptr()); }
        println!("gpu build per animated frame (top bar 4K, buttons sliding): {:.2} ms, {} vertices", t.elapsed().as_secs_f64() * 1000.0 / n as f64, c[0]);
        vitrail_destroy(ui);
    }
}

#[test]
fn compositor_frame_cost() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/pages/game-hud");
    let html = std::fs::read_to_string(dir.join("topbar.html")).unwrap().replace("<body>", "<body class=\"founded\">");
    let base = format!("file://{}/topbar.html", dir.display());
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new(base).unwrap().as_ptr(), 3840, 323, 2.15);
        vitrail_set_gpu(ui, 2);
        let mut c = [0u32; 7];
        vitrail_tick(ui, 0.0);
        vitrail_gpu_build(ui, c.as_mut_ptr());
        let sel = CString::new(".btns").unwrap();
        vitrail_set_style(ui, sel.as_ptr(), CString::new("transition").unwrap().as_ptr(), CString::new("transform 20s linear").unwrap().as_ptr());
        vitrail_set_style(ui, sel.as_ptr(), CString::new("transform").unwrap().as_ptr(), CString::new("translateX(-600px)").unwrap().as_ptr());
        vitrail_tick(ui, 0.05);
        vitrail_gpu_build(ui, c.as_mut_ptr());
        let mut st = vec![0f32; 7 * 8];
        let (n, mut composited) = (120, 0);
        let t = Instant::now();
        for i in 0..n {
            if vitrail_tick(ui, 0.1 + i as f64 / 60.0) == 2 { composited += 1; } else { vitrail_gpu_build(ui, c.as_mut_ptr()); }
            vitrail_layer_states(ui, st.as_mut_ptr(), st.len());
        }
        println!("compositor frame (top bar 4K, buttons sliding): {:.3} ms, {composited}/{n} frames without painting", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        vitrail_destroy(ui);
    }
}

#[test]
fn gpu_hover_upload() {
    let big = std::env::var_os("BIG").is_some();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/pages/game-hud");
    let (html, base, w, h, s) = if big {
        let page = format!("{}/../../tests/pages/30-inventory.html", env!("CARGO_MANIFEST_DIR"));
        (std::fs::read_to_string(&page).unwrap(), format!("file://{page}"), 1920, 1080, 1.0)
    } else {
        (std::fs::read_to_string(dir.join("topbar.html")).unwrap().replace("<body>", "<body class=\"founded\">"), format!("file://{}/topbar.html", dir.display()), 3840, 323, 2.15)
    };
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new(base).unwrap().as_ptr(), w, h, s);
        vitrail_set_gpu(ui, 2);
        let mut regions = 0u32;
        let mut rect = [0u32; 4];
        let paint = |ui: *mut Vitrail, rect: &mut [u32; 4], regions: &mut u32| {
            vitrail_paint_start(ui);
            while vitrail_paint_poll(ui, std::ptr::null_mut(), 0, rect.as_mut_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), 0, regions) == 0 { std::thread::yield_now(); }
        };
        paint(ui, &mut rect, &mut regions);
        let mut c = [0u32; 7];
        let mut ch = [0u32; 4];
        let spots: Vec<(f32, f32)> = if big { vec![(90.0, 90.0), (330.0, 90.0), (1500.0, 600.0), (900.0, 900.0)] } else { vec![(3700.0, 60.0), (3500.0, 60.0), (100.0, 80.0), (900.0, 50.0)] };
        for (i, (x, y)) in spots.into_iter().enumerate() {
            if big {
                // A real change: one cell's background (cells 5, 60, 120, 200 of the 250 on screen).
                let sel = CString::new(format!("#it{}", [5, 60, 120, 200][i])).unwrap();
                vitrail_set_style(ui, sel.as_ptr(), CString::new("background").unwrap().as_ptr(), CString::new("#803030").unwrap().as_ptr());
            } else {
                vitrail_pointer_move(ui, x, y);
            }
            paint(ui, &mut rect, &mut regions);
            vitrail_gpu_counts(ui, c.as_mut_ptr());
            vitrail_gpu_changed(ui, ch.as_mut_ptr());
            let mut sp = [0u32; 64];
            let ns = vitrail_gpu_vertex_spans(ui, sp.as_mut_ptr(), 32) as usize;
            let written: u32 = (0..ns).map(|k| sp[k * 2 + 1] - sp[k * 2]).sum();
            let mut dr = [0f32; 16];
            let nd = vitrail_gpu_dirty(ui, dr.as_mut_ptr(), 4) as usize;
            let share: f32 = (0..nd).map(|k| (dr[k * 4 + 2] - dr[k * 4]) * (dr[k * 4 + 3] - dr[k * 4 + 1])).sum::<f32>() * 100.0 / (w * h) as f32;
            println!("  dirty: {nd} rectangles, {share:.1}% of the page");
            println!("  {ns} spans, {written} vertices written ({:.1}%)", written as f64 * 100.0 / c[0].max(1) as f64);
            println!("hover at {x}: vertices changed {}..{} of {} ({:.0}%), indices {}..{} of {}", ch[0], ch[1], c[0], (ch[1] - ch[0]) as f64 * 100.0 / c[0].max(1) as f64, ch[2], ch[3], c[1]);
        }
        vitrail_destroy(ui);
    }
}

/// A big page: an inventory of 2000 cells in a scrolled window, full HD.
#[test]
fn big_page_cost() {
    let page = format!("{}/../../tests/pages/30-inventory.html", env!("CARGO_MANIFEST_DIR"));
    let html = std::fs::read_to_string(&page).unwrap();
    unsafe {
        let t = Instant::now();
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new(format!("file://{page}")).unwrap().as_ptr(), 1920, 1080, 1.0);
        let created = t.elapsed();
        vitrail_set_gpu(ui, 2);
        let mut c = [0u32; 7];
        let t = Instant::now();
        vitrail_gpu_build(ui, c.as_mut_ptr());
        println!("big page: create {:.0} ms, first GPU build {:.1} ms, {} vertices, {} batches, {} ops", created.as_secs_f64() * 1e3, t.elapsed().as_secs_f64() * 1e3, c[0], c[2], c[5]);
        // Hover over different cells.
        let n = 20;
        let t = Instant::now();
        for i in 0..n { vitrail_pointer_move(ui, 80.0 + (i % 10) as f32 * 120.0, 90.0 + (i / 10) as f32 * 128.0); vitrail_gpu_build(ui, c.as_mut_ptr()); }
        println!("big page: per hover {:.2} ms (whole build)", t.elapsed().as_secs_f64() * 1e3 / n as f64);
        // Scroll the window.
        let t = Instant::now();
        let mut st = [0f32; 512];
        let (mut gpu_only, steps) = (0, 60);
        for _ in 0..steps {
            // 40 px a wheel step: within the held margin the GPU moves the content (2), past it the page paints.
            let t1 = Instant::now();
            let r = vitrail_scroll(ui, 900.0, 500.0, 0.0, 40.0);
            let ts = t1.elapsed();
            if r == 2 { gpu_only += 1; vitrail_scroll_states(ui, st.as_mut_ptr(), 256); if std::env::var_os("TRACE").is_some() { eprintln!("gpu step: scroll {:.3} ms", ts.as_secs_f64() * 1e3); } }
            else { let t2 = Instant::now(); vitrail_gpu_build(ui, c.as_mut_ptr()); if std::env::var_os("TRACE").is_some() { eprintln!("repaint: scroll {:.3} ms, build {:.1} ms", ts.as_secs_f64() * 1e3, t2.elapsed().as_secs_f64() * 1e3); } }
        }
        println!("big page: per scroll step {:.3} ms, {gpu_only}/{steps} steps without painting, {} vertices", t.elapsed().as_secs_f64() * 1e3 / steps as f64, c[0]);
        vitrail_destroy(ui);
    }
}

/// For profiling (LOOP=n): rebuilds the big page n times, hovering between two cells.
#[test]
fn big_page_loop() {
    let Some(n) = std::env::var("LOOP").ok().and_then(|v| v.parse::<usize>().ok()) else { return };
    let page = format!("{}/../../tests/pages/30-inventory.html", env!("CARGO_MANIFEST_DIR"));
    let html = std::fs::read_to_string(&page).unwrap();
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new(format!("file://{page}")).unwrap().as_ptr(), 1920, 1080, 1.0);
        vitrail_set_gpu(ui, 2);
        let mut c = [0u32; 7];
        for i in 0..n {
            let sel = CString::new(format!("#it{}", 5 + (i % 2) * 50)).unwrap();
            vitrail_set_style(ui, sel.as_ptr(), CString::new("background").unwrap().as_ptr(), CString::new(if i % 4 < 2 { "#803030" } else { "#305080" }).unwrap().as_ptr());
            vitrail_gpu_build(ui, c.as_mut_ptr());
        }
        vitrail_destroy(ui);
    }
}
