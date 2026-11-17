//! The performance targets (M4, 4K): a change or hover of one element ≤ 0.3 ms on the main thread on any page, 20
//! animations ≤ 0.3 ms a frame, a full-screen window of 100 cards ≤ 3 ms. Each case prints where the time goes:
//! styles and layout, the draw list (main thread), the GPU frame (worker).
//! Run: cargo test --release --test perf -- --nocapture --test-threads 1
use std::ffi::CString;
use vitrail::*;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

/// Runs `change(i)` then a full GPU build, `n` times after a warm-up; prints the means and returns the main thread's.
unsafe fn case(name: &str, ui: *mut Vitrail, n: usize, mut change: impl FnMut(usize)) -> f64 {
    unsafe {
        vitrail_set_gpu(ui, 2);
        let mut c = [0u32; 7];
        let mut p = [0f64; 4];
        for i in 0..10 { change(i); vitrail_gpu_build(ui, c.as_mut_ptr()); }
        let (mut style, mut paint, mut build, mut cmds, mut input) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for i in 0..n {
            let t = std::time::Instant::now();
            change(10 + i);
            input += t.elapsed().as_secs_f64() * 1e3;
            vitrail_gpu_build(ui, c.as_mut_ptr());
            vitrail_profile(ui, p.as_mut_ptr());
            style += p[0]; paint += p[1]; build += p[2]; cmds = p[3];
        }
        let k = n as f64;
        let main = (input + style + paint) / k;
        println!("{name}: main {main:.3} ms (the change {:.3}, styles+layout {:.3}, draw list {:.3}) · worker {:.3} ms · {cmds} commands, {} vertices", input / k, style / k, paint / k, build / k, c[0]);
        main
    }
}

fn topbar() -> (String, String) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/pages/game-hud");
    (std::fs::read_to_string(dir.join("topbar.html")).unwrap().replace("<body>", "<body class=\"founded\">"), format!("file://{}/topbar.html", dir.display()))
}

fn cards(n: usize, anim: &str) -> String {
    let mut html = format!(r#"<!doctype html><html><head><style>
html,body{{margin:0;background:#16241f;font:600 15px sans-serif;color:#eee}}
.grid{{display:grid;grid-template-columns:repeat(12,1fr);gap:14px;padding:20px}}
.card{{height:120px;border-radius:12px;background:linear-gradient(#2a4a43,#0c1b18);box-shadow:0 4px 10px rgba(0,0,0,.5),inset 0 0 0 1px #5b7d74;padding:10px}}
.card:hover,.card.hot{{background:linear-gradient(#3a6a5f,#14302a)}}
{anim}
</style></head><body><div class="grid">"#);
    for i in 0..n { html.push_str(&format!("<div class=\"card\" id=\"c{i}\">Знання {i}<br>Відкриває будівлю</div>")); }
    html.push_str("</div></body></html>");
    html
}

#[test]
fn targets() {
    unsafe {
        let mut results = Vec::new();
        // 1. The top bar: hovering its buttons and rings.
        let (html, base) = topbar();
        let ui = vitrail_create(s(&html).as_ptr(), s(&base).as_ptr(), 3840, 323, 2.15);
        let spots = [(3700.0, 60.0), (3500.0, 60.0), (100.0, 80.0), (900.0, 50.0)];
        results.push(("top bar hover", case("top bar hover", ui, 40, |i| { let (x, y) = spots[i % 4]; vitrail_pointer_move(ui, x, y); }), 0.3));
        vitrail_destroy(ui);

        // 2. The 2000-cell inventory: one cell's colour.
        let page = format!("{}/../../tests/pages/30-inventory.html", env!("CARGO_MANIFEST_DIR"));
        let ui = vitrail_create(s(&std::fs::read_to_string(&page).unwrap()).as_ptr(), s(&format!("file://{page}")).as_ptr(), 1920, 1080, 1.0);
        results.push(("inventory, one cell", case("inventory (2000 cells), one cell changes", ui, 20, |i| {
            vitrail_set_style(ui, s(&format!("#it{}", 5 + (i % 2) * 50)).as_ptr(), s("background").as_ptr(), s(if i % 4 < 2 { "#803030" } else { "#305080" }).as_ptr());
        }), 0.3));
        vitrail_destroy(ui);

        // 2b. The inventory scrolled by the wheel: within the held content the GPU moves it (no paint); a step past it
        // paints. The mean over 120 steps of 40 px, and the dearest step.
        let ui = vitrail_create(s(&std::fs::read_to_string(&page).unwrap()).as_ptr(), s(&format!("file://{page}")).as_ptr(), 1920, 1080, 1.0);
        vitrail_set_gpu(ui, 2);
        let mut c = [0u32; 7];
        vitrail_gpu_build(ui, c.as_mut_ptr());
        let (mut total, mut worst, mut painted) = (0.0f64, 0.0f64, 0);
        for i in 0..120 {
            let t = std::time::Instant::now();
            vitrail_scroll(ui, 900.0, 500.0, 0.0, if (i / 40) % 2 == 0 { 40.0 } else { -40.0 });
            if vitrail_needs_paint(ui) != 0 { vitrail_gpu_build(ui, c.as_mut_ptr()); painted += 1; }
            let ms = t.elapsed().as_secs_f64() * 1e3;
            total += ms; worst = worst.max(ms);
        }
        let mean = total / 120.0;
        println!("inventory (2000 cells), wheel steps: mean {mean:.3} ms, dearest {worst:.2} ms (a step past the held content), {painted}/120 painted");
        results.push(("inventory, a wheel step (mean)", mean, 0.3));
        vitrail_destroy(ui);

        // 3. A full-screen 4K window of 100 cards: hovering cards (the whole change ≤ 3 ms, main ≤ 0.3).
        let ui = vitrail_create(s(&cards(100, "")).as_ptr(), s("").as_ptr(), 3840, 2160, 2.0);
        results.push(("100-card window hover", case("4K window of 100 cards, hover", ui, 30, |i| { vitrail_set_class(ui, s(&format!("#c{}", (i * 7) % 100)).as_ptr(), s("hot").as_ptr(), (i / 2 % 2) as i32); }), 0.3));
        vitrail_destroy(ui);

        // 4. 20 cards changing colour at once (a frame of the clock).
        let ui = vitrail_create(s(&cards(20, ".card{animation:a 2s infinite alternate} @keyframes a{from{background:#2a4a43}to{background:#6a3a23}}")).as_ptr(), s("").as_ptr(), 3840, 2160, 2.0);
        results.push(("20 colour animations", case("20 cards changing colour", ui, 60, |i| { vitrail_tick(ui, 1.0 + i as f64 / 60.0); }), 0.3));
        vitrail_destroy(ui);

        // Over the target fails with STRICT=1; twice the target always fails (a regression, not noise).
        let strict = std::env::var_os("STRICT").is_some();
        for (name, main, target) in &results {
            let ok = main <= target;
            println!("{} {name}: {main:.3} ms (target {target} ms)", if ok { "✅" } else { "❌" });
            // PERF_SLACK: how far over the target a slower machine (a shared CI runner) may go before it fails.
            let slack: f64 = std::env::var("PERF_SLACK").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0);
            assert!(*main <= target * if strict { 1.0 } else { slack }, "{name} {main:.3} ms over {target} ms");
        }
    }
}
