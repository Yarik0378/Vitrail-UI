//! Retained painting (Blitz reuses unchanged elements' recordings) draws exactly what painting everything draws:
//! after each kind of change, the page painted with reuse equals, pixel for pixel, the same page painted afresh.
use std::ffi::{CString, c_char};
use vitrail::*;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

unsafe fn pixels(ui: *mut Vitrail, w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    unsafe { vitrail_render(ui, px.as_mut_ptr(), px.len()) };
    px
}

/// The page painted with reuse, and painted afresh, must match.
unsafe fn same(ui: *mut Vitrail, w: u32, h: u32, what: &str) {
    unsafe {
        let kept = pixels(ui, w, h);
        std::env::set_var("BLITZ_NO_RETAIN", "1");
        let fresh = pixels(ui, w, h);
        std::env::remove_var("BLITZ_NO_RETAIN");
        let diff = kept.iter().zip(&fresh).filter(|(a, b)| a != b).count();
        if diff > 0 {
            let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
            for (i, (a, b)) in kept.chunks(4).zip(fresh.chunks(4)).enumerate() { if a != b { let (x, y) = (i as u32 % w, i as u32 / w); x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y); } }
            eprintln!("{what}: differs in x {x0}..{x1}, y {y0}..{y1}");
        }
        assert_eq!(diff, 0, "{what}: {diff} bytes differ between reused and fresh painting");
    }
}

unsafe fn click(ui: *mut Vitrail, x: f32, y: f32) {
    unsafe {
        vitrail_pointer_move(ui, x, y);
        vitrail_pointer_down(ui, x, y);
        let (mut a, mut b) = ([0 as c_char; 64], [0 as c_char; 64]);
        vitrail_pointer_up(ui, x, y, a.as_mut_ptr(), b.as_mut_ptr(), 64);
    }
}

#[test]
fn reused_painting_matches_fresh_painting() {
    unsafe {
        // The inventory: cell colours, hover, a scroll.
        let page = format!("{}/../../tests/pages/30-inventory.html", env!("CARGO_MANIFEST_DIR"));
        let (w, h) = (1200, 700);
        let ui = vitrail_create(s(&std::fs::read_to_string(&page).unwrap()).as_ptr(), s(&format!("file://{page}")).as_ptr(), w, h, 1.0);
        same(ui, w, h, "inventory, first paint");
        for i in 0..4 {
            vitrail_set_style(ui, s(&format!("#it{}", 3 + i * 7)).as_ptr(), s("background").as_ptr(), s(if i % 2 == 0 { "#803030" } else { "#305080" }).as_ptr());
            same(ui, w, h, "inventory, a cell's colour");
        }
        vitrail_pointer_move(ui, 200.0, 150.0); same(ui, w, h, "inventory, hover");
        vitrail_pointer_move(ui, 600.0, 400.0); same(ui, w, h, "inventory, hover moved");
        vitrail_scroll(ui, 600.0, 400.0, 0.0, 300.0); same(ui, w, h, "inventory, scrolled");
        vitrail_destroy(ui);

        // The top bar: classes, text, styles, hover.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/pages/game-hud");
        let html = std::fs::read_to_string(dir.join("topbar.html")).unwrap();
        let (w, h) = (1422, 150);
        let ui = vitrail_create(s(&html).as_ptr(), s(&format!("file://{}/topbar.html", dir.display())).as_ptr(), w, h, 1.0);
        same(ui, w, h, "top bar, before founding");
        vitrail_set_class(ui, s("body").as_ptr(), s("founded").as_ptr(), 1); same(ui, w, h, "top bar, founded");
        vitrail_set_text(ui, s("#wood").as_ptr(), s("123").as_ptr()); same(ui, w, h, "top bar, a number");
        vitrail_set_style(ui, s("#ring-science").as_ptr(), s("--p").as_ptr(), s("40%").as_ptr()); same(ui, w, h, "top bar, a ring's progress");
        vitrail_set_class(ui, s("#cell-iron").as_ptr(), s("off").as_ptr(), 1); same(ui, w, h, "top bar, a cell hidden (layout shifts)");
        vitrail_pointer_move(ui, 1380.0, 30.0); same(ui, w, h, "top bar, a button hovered");
        vitrail_destroy(ui);

        // Scripts changing the DOM: petite-vue and Preact.
        for (name, target) in [("40-petite-vue.html", (100.0, 85.0)), ("41-preact.html", (100.0, 130.0))] {
            let page = format!("{}/../../tests/pages/{name}", env!("CARGO_MANIFEST_DIR"));
            let (w, h) = (450, 320);
            let ui = vitrail_create(s(&std::fs::read_to_string(&page).unwrap()).as_ptr(), s(&format!("file://{page}")).as_ptr(), w, h, 1.0);
            same(ui, w, h, name);
            for _ in 0..3 { click(ui, target.0, target.1); vitrail_tick(ui, 0.5); same(ui, w, h, &format!("{name}, after a click")); }
            vitrail_js_eval(ui, s("const t = document.querySelector('li, .cell'); if (t) t.firstChild.data = 'changed text';").as_ptr());
            same(ui, w, h, &format!("{name}, a text node's data"));
            vitrail_destroy(ui);
        }

        // A scrolled list inside a page (scrolling damages nothing).
        let (w, h) = (300, 120);
        let list = "<div id=l style='height:80px;width:200px;overflow:auto'>".to_string() + &(0..30).map(|i| format!("<div style='height:20px;background:hsl({},50%,40%)'>row {i}</div>", i * 12)).collect::<String>() + "</div>";
        let ui = vitrail_create(s(&list).as_ptr(), s("").as_ptr(), w, h, 1.0);
        same(ui, w, h, "a list");
        for _ in 0..3 { vitrail_scroll(ui, 100.0, 40.0, 0.0, 45.0); same(ui, w, h, "a list scrolled"); }
        vitrail_destroy(ui);

        // A text field being typed into (its editor changes without damage).
        let (w, h) = (300, 80);
        let ui = vitrail_create(s("<input id=f style='width:200px' value='abc'>").as_ptr(), s("").as_ptr(), w, h, 1.0);
        click(ui, 50.0, 12.0);
        for k in ["x", "y", "z"] { vitrail_key(ui, 1, s(k).as_ptr(), s("").as_ptr(), 0, 0); same(ui, w, h, "a text field typed into"); }
        vitrail_destroy(ui);
    }
}

/// In GPU mode (composited layers, scrolling on the GPU), the recorded commands with reuse equal those painted afresh.
unsafe fn same_commands(ui: *mut Vitrail, what: &str) {
    let kept = record_page(ui);
    // A fresh recording of the same state: nothing reused (and the damage it would consume is already consumed).
    unsafe { std::env::set_var("BLITZ_NO_RETAIN", "1"); }
    let fresh = record_page(ui);
    unsafe { std::env::remove_var("BLITZ_NO_RETAIN"); }
    if kept.len() != fresh.len() {
        let first = kept.iter().zip(&fresh).position(|(a, b)| a != b).unwrap_or(kept.len().min(fresh.len()));
        let show = |v: &Vec<_>, i: usize| v.get(i).map(|c| format!("{c:?}").chars().take(200).collect::<String>()).unwrap_or_default();
        eprintln!("{what}: kept {} fresh {}; first difference {first}:\n kept  {}\n fresh {}", kept.len(), fresh.len(), show(&kept, first), show(&fresh, first));
    }
    assert_eq!(kept.len(), fresh.len(), "{what}: command counts differ");
    let first = kept.iter().zip(&fresh).position(|(a, b)| a != b);
    if let Some(i) = first { eprintln!("{what}: kept {:?}\n fresh {:?}", format!("{:?}", kept[i]).chars().take(300).collect::<String>(), format!("{:?}", fresh[i]).chars().take(300).collect::<String>()); }
    assert!(first.is_none(), "{what}: command {first:?} differs");
}

#[test]
fn reused_commands_match_in_gpu_mode() {
    unsafe {
        // The inventory scrolled past the held margin, cells changed.
        let page = format!("{}/../../tests/pages/30-inventory.html", env!("CARGO_MANIFEST_DIR"));
        let ui = vitrail_create(s(&std::fs::read_to_string(&page).unwrap()).as_ptr(), s(&format!("file://{page}")).as_ptr(), 1920, 1080, 1.0);
        vitrail_set_gpu(ui, 2);
        same_commands(ui, "inventory");
        for i in 0..3 {
            vitrail_set_style(ui, s(&format!("#it{}", 4 + i * 9)).as_ptr(), s("background").as_ptr(), s("#702020").as_ptr());
            same_commands(ui, "inventory, a cell");
            vitrail_scroll(ui, 900.0, 500.0, 0.0, 700.0);
            same_commands(ui, "inventory, scrolled");
        }
        vitrail_destroy(ui);
        // A transform animation (composited) starting and ending.
        let page = "<style>#a{width:60px;height:30px;background:#386;transition:transform .2s linear}#a.go{transform:translateX(40px)}</style><div id=a></div><div style='width:80px;height:20px;background:#a33'></div>";
        let ui = vitrail_create(s(page).as_ptr(), s("").as_ptr(), 300, 120, 2.0);
        vitrail_set_gpu(ui, 2);
        vitrail_tick(ui, 0.0);
        same_commands(ui, "before the animation");
        vitrail_set_class(ui, s("#a").as_ptr(), s("go").as_ptr(), 1);
        for i in 1..=12 { vitrail_tick(ui, i as f64 * 0.025); same_commands(ui, "during and after the animation"); }
        vitrail_destroy(ui);
    }
}

/// The frame as triangles: every index resolved to its vertex's bytes, the batches (material, texture, count) and the
/// ops. Places in the vertex buffer may differ between frames; what is drawn must not.
unsafe fn drawn(ui: *mut Vitrail) -> (Vec<[u8; 60]>, Vec<[u32; 3]>, Vec<u8>) {
    unsafe {
        let mut c = [0u32; 7];
        vitrail_gpu_build(ui, c.as_mut_ptr());
        let (nv, ni, nb, nops) = (c[0] as usize, c[1] as usize, c[2] as usize, c[5] as usize);
        let mut v = vec![0u8; nv * 60];
        let mut idx = vec![0u32; ni];
        let mut b = vec![0u8; nb * 256];
        vitrail_gpu_copy(ui, v.as_mut_ptr(), v.len(), idx.as_mut_ptr(), idx.len(), b.as_mut_ptr(), b.len());
        let tris = idx.iter().map(|&i| v[i as usize * 60..i as usize * 60 + 60].try_into().unwrap()).collect();
        let bsz = b.len() / nb.max(1);
        let _ = bsz;
        let mut ops = vec![0u8; nops * 64];
        vitrail_gpu_ops(ui, ops.as_mut_ptr(), ops.len());
        let batches = (0..nb).map(|k| { let o = k * std::mem::size_of::<vitrail::gpu::Batch>(); let w = |j: usize| u32::from_le_bytes(b[o + j * 4..o + j * 4 + 4].try_into().unwrap()); [w(0), w(2), w(3)] }).collect();
        (tris, batches, ops)
    }
}

#[test]
fn replayed_groups_draw_the_same_frame() {
    unsafe {
        let page = format!("{}/../../tests/pages/30-inventory.html", env!("CARGO_MANIFEST_DIR"));
        let ui = vitrail_create(s(&std::fs::read_to_string(&page).unwrap()).as_ptr(), s(&format!("file://{page}")).as_ptr(), 1920, 1080, 1.0);
        vitrail_set_gpu(ui, 2);
        let check = |what: &str| {
            let kept = drawn(ui);
            vitrail_set_style(ui, s("body").as_ptr(), s("--z").as_ptr(), s(what).as_ptr()); // a change that forces a paint
            std::env::set_var("VITRAIL_NO_GROUPS", "1");
            let fresh = drawn(ui);
            std::env::remove_var("VITRAIL_NO_GROUPS");
            assert_eq!(kept.0.len(), fresh.0.len(), "{what}: triangle counts");
            let first = kept.0.iter().zip(&fresh.0).position(|(a, b)| a != b);
            assert!(first.is_none(), "{what}: index {first:?} draws another vertex");
            assert_eq!(kept.1, fresh.1, "{what}: batches");
            assert_eq!(kept.2, fresh.2, "{what}: ops");
        };
        check("first");
        for i in 0..4 {
            vitrail_set_style(ui, s(&format!("#it{}", 5 + i * 11)).as_ptr(), s("background").as_ptr(), s(if i % 2 == 0 { "#803030" } else { "#305080" }).as_ptr());
            check("a cell");
        }
        vitrail_pointer_move(ui, 300.0, 200.0); check("hover");
        vitrail_scroll(ui, 900.0, 500.0, 0.0, 900.0); check("scrolled past the margin");
        vitrail_destroy(ui);
        // The top bar (glass, clips, gradients) with a value changing.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/pages/game-hud");
        let html = std::fs::read_to_string(dir.join("topbar.html")).unwrap().replace("<body>", "<body class=\"founded\">");
        let ui = vitrail_create(s(&html).as_ptr(), s(&format!("file://{}/topbar.html", dir.display())).as_ptr(), 3840, 323, 2.15);
        vitrail_set_gpu(ui, 2);
        let check = |what: &str| {
            let kept = drawn(ui);
            vitrail_set_style(ui, s("body").as_ptr(), s("--z").as_ptr(), s(what).as_ptr());
            std::env::set_var("VITRAIL_NO_GROUPS", "1");
            let fresh = drawn(ui);
            std::env::remove_var("VITRAIL_NO_GROUPS");
            assert_eq!(kept.0, fresh.0, "{what}: triangles");
            assert_eq!(kept.1, fresh.1, "{what}: batches");
        };
        check("top bar");
        vitrail_set_text(ui, s("#wood").as_ptr(), s("77").as_ptr()); check("a number");
        vitrail_pointer_move(ui, 3700.0, 60.0); check("a button hovered");
        vitrail_destroy(ui);
    }
}
