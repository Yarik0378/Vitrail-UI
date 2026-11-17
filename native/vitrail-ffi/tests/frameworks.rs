//! UI libraries run unchanged: petite-vue and Preact (with hooks and htm) render, react to clicks from the pointer and
//! to calls from the game, and report back through `engine.trigger`. Pages: tests/pages/40-petite-vue.html,
//! 41-preact.html (the libraries in tests/pages/vendor).
use std::ffi::{CString, c_char};
use vitrail::*;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

fn page(name: &str) -> (String, String) {
    let path = format!("{}/../../tests/pages/{name}", env!("CARGO_MANIFEST_DIR"));
    (std::fs::read_to_string(&path).unwrap(), format!("file://{path}"))
}

unsafe fn errors(ui: *mut Vitrail) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut level = 0;
    while unsafe { vitrail_js_log(ui, buf.as_mut_ptr() as *mut c_char, buf.len(), &mut level) } == 1 {
        let n = buf.iter().position(|&b| b == 0).unwrap();
        let m = String::from_utf8_lossy(&buf[..n]).into_owned();
        println!("  console[{level}] {m}");
        if level >= 1 { out.push(m); }
    }
    out
}

unsafe fn events(ui: *mut Vitrail) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut a, mut b) = (vec![0u8; 256], vec![0u8; 1 << 16]);
    while unsafe { vitrail_js_poll(ui, a.as_mut_ptr() as *mut c_char, a.len(), b.as_mut_ptr() as *mut c_char, b.len()) } == 1 {
        let (na, nb) = (a.iter().position(|&x| x == 0).unwrap(), b.iter().position(|&x| x == 0).unwrap());
        out.push((String::from_utf8_lossy(&a[..na]).into_owned(), String::from_utf8_lossy(&b[..nb]).into_owned()));
    }
    out
}

/// The value of a JavaScript expression in the page, as JSON.
unsafe fn probe(ui: *mut Vitrail, expr: &str) -> String {
    unsafe {
        events(ui);
        vitrail_js_eval(ui, s(&format!("engine.trigger('probe', {expr})")).as_ptr());
        let ev = events(ui);
        let (_, json) = ev.into_iter().rev().find(|(n, _)| n == "probe").expect("probe answered");
        json.trim_start_matches('[').trim_end_matches(']').to_string()
    }
}

/// A real click (pointer down and up) in the middle of the first element matching `sel`.
unsafe fn click(ui: *mut Vitrail, sel: &str) {
    unsafe {
        let mut r = [0f32; 4];
        assert_eq!(vitrail_rect(ui, s(sel).as_ptr(), r.as_mut_ptr()), 1, "{sel} is on the page");
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        vitrail_pointer_move(ui, x, y);
        vitrail_pointer_down(ui, x, y);
        let (mut a, mut b) = ([0 as c_char; 64], [0 as c_char; 64]);
        vitrail_pointer_up(ui, x, y, a.as_mut_ptr(), b.as_mut_ptr(), 64);
    }
}

#[test]
fn petite_vue_runs_unchanged() {
    unsafe {
        let (html, base) = page("40-petite-vue.html");
        let ui = vitrail_create(s(&html).as_ptr(), s(&base).as_ptr(), 900, 500, 2.0);
        assert!(errors(ui).is_empty(), "no errors while loading");
        assert!(events(ui).iter().any(|(n, _)| n == "mounted"));
        assert_eq!(probe(ui, "document.querySelector('#count').textContent"), "\"0\"");
        assert_eq!(probe(ui, "[...document.querySelectorAll('#list li')].map(l => l.textContent + (l.className ? ':' + l.className : '')).join(',')"), "\"wood,stone:done,iron\"");
        assert_eq!(probe(ui, "!!document.querySelector('#even') && !document.querySelector('#odd')"), "true");

        click(ui, "#inc");
        assert!(events(ui).contains(&("count".into(), "[1]".into())), "the click reached petite-vue's handler");
        assert_eq!(probe(ui, "document.querySelector('#count').textContent"), "\"1\"");
        assert_eq!(probe(ui, "!document.querySelector('#even') && !!document.querySelector('#odd')"), "true", "v-if / v-else switched");

        assert_eq!(vitrail_js_call(ui, s("add").as_ptr(), s("[\"gold\"]").as_ptr()), 1);
        assert_eq!(probe(ui, "document.querySelectorAll('#list li').length"), "4", "the game's call grew the list");
        click(ui, "#list li");
        assert_eq!(probe(ui, "document.querySelector('#list li').className"), "\"done\"", ":class follows the state");
        assert!(errors(ui).is_empty(), "no errors while used");
        vitrail_destroy(ui);
    }
}

#[test]
fn preact_runs_unchanged() {
    unsafe {
        let (html, base) = page("41-preact.html");
        let ui = vitrail_create(s(&html).as_ptr(), s(&base).as_ptr(), 900, 500, 2.0);
        // useEffect runs after the first paint: Preact schedules it on an animation frame or a timer.
        vitrail_tick(ui, 0.05);
        vitrail_tick(ui, 0.2);
        assert!(errors(ui).is_empty(), "no errors while loading");
        assert!(events(ui).iter().any(|(n, _)| n == "ready"), "useEffect ran");
        assert_eq!(probe(ui, "document.querySelector('#turn').textContent"), "\"Turn 1\"");
        assert_eq!(probe(ui, "document.querySelector('#turn').style.fontSize"), "\"24px\"", "a style object");
        assert_eq!(probe(ui, "document.querySelector('#res-stone').className"), "\"cell low\"");

        click(ui, "#end");
        assert!(events(ui).contains(&("endTurn".into(), "[1]".into())), "onClick ran");
        click(ui, "#end");
        vitrail_tick(ui, 0.3);
        assert_eq!(probe(ui, "document.querySelector('#turn').textContent"), "\"Turn 3\"");
        assert_eq!(probe(ui, "!!document.querySelector('#late')"), "true", "a conditional element appeared");

        vitrail_js_call(ui, s("stock").as_ptr(), s("[{\"wood\":3,\"stone\":50}]").as_ptr());
        vitrail_tick(ui, 0.4);
        assert_eq!(probe(ui, "document.querySelector('#res-wood').className + '|' + document.querySelector('#res-stone').className"), "\"cell low|cell\"");
        assert!(errors(ui).is_empty(), "no errors while used");
        vitrail_destroy(ui);
    }
}

/// What JavaScript costs on the main thread: loading each page (scripts included) and one click's update, the mean of
/// 50. Budgets: the scripts' share of loading under 30 ms, a click under 1 ms (run with --nocapture to see the numbers).
#[test]
fn script_costs() {
    unsafe {
        for (name, target) in [("40-petite-vue.html", "#inc"), ("41-preact.html", "#end")] {
            let (html, base) = page(name);
            // The same page without its scripts first (fonts and styles load then), so `load` is the scripts' share.
            let plain = html.replace("<script", "<template").replace("</script>", "</template>");
            let t = std::time::Instant::now();
            vitrail_destroy(vitrail_create(s(&plain).as_ptr(), s(&base).as_ptr(), 900, 500, 2.0));
            let t0 = std::time::Instant::now();
            let ui = vitrail_create(s(&html).as_ptr(), s(&base).as_ptr(), 900, 500, 2.0);
            let total = t0.elapsed().as_secs_f64() * 1000.0;
            let t1 = std::time::Instant::now();
            vitrail_destroy(vitrail_create(s(&plain).as_ptr(), s(&base).as_ptr(), 900, 500, 2.0));
            let base_ms = t1.elapsed().as_secs_f64() * 1000.0;
            let _ = t;
            let load = total - base_ms;
            let mut r = [0f32; 4];
            vitrail_rect(ui, s(target).as_ptr(), r.as_mut_ptr());
            let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
            let t = std::time::Instant::now();
            for i in 0..50 {
                vitrail_pointer_down(ui, x, y);
                let (mut a, mut b) = ([0 as c_char; 64], [0 as c_char; 64]);
                vitrail_pointer_up(ui, x, y, a.as_mut_ptr(), b.as_mut_ptr(), 64);
                vitrail_tick(ui, 0.1 + i as f64 * 0.02);
            }
            let click = t.elapsed().as_secs_f64() * 1000.0 / 50.0;
            println!("{name}: page {total:.1} ms, of which scripts {load:.1} ms; a click and its update {click:.3} ms");
            assert!(load < 30.0 && click < 1.0, "{name}: load {load:.1} ms, click {click:.3} ms");
            events(ui);
            vitrail_destroy(ui);
        }
    }
}


#[test]
fn es_modules_with_an_import_map() {
    unsafe {
        let (html, base) = page("42-preact-modules.html");
        let ui = vitrail_create(s(&html).as_ptr(), s(&base).as_ptr(), 900, 500, 2.0);
        assert!(errors(ui).is_empty(), "no errors while loading");
        assert!(events(ui).contains(&("modules".into(), "[3]".into())), "the module ran to its end (JSON import, top-level await)");
        assert_eq!(probe(ui, "document.querySelector('#title').textContent"), "\"Stock (awaited)\"");
        assert_eq!(probe(ui, "[...document.querySelectorAll('.cell')].map(c => c.textContent + ':' + c.className).join(',')"), "\"wood: 12:cell,stone: 3:cell low,iron: 0:cell low\"");
        vitrail_destroy(ui);
    }
}

#[test]
fn module_errors_are_reported() {
    unsafe {
        let html = "<script type=module>import x from 'nowhere'; </script><script type=module>import y from './missing.js';</script><script type=module>await Promise.reject(new Error('late'));</script>";
        let ui = vitrail_create(s(html).as_ptr(), s(&format!("file://{}/../../tests/pages/x.html", env!("CARGO_MANIFEST_DIR"))).as_ptr(), 100, 100, 1.0);
        let e = errors(ui);
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(e[0].contains("nowhere") && e[1].contains("missing.js") && e[2].contains("late"), "{e:?}");
        vitrail_destroy(ui);
    }
}

#[test]
fn data_binding_without_scripts() {
    unsafe {
        let (html, base) = page("43-binding.html");
        let ui = vitrail_create(s(&html).as_ptr(), s(&base).as_ptr(), 900, 500, 2.0);
        let model = |name: &str, json: &str| { vitrail_js_call(ui, s("__model").as_ptr(), s(&format!("[\"{name}\",{json}]")).as_ptr()); };
        model("game", r#"{"turn":3,"season":"spring"}"#);
        model("player", r#"{"gold":25,"hp":0.5,"name":"Ada","items":[{"name":"axe","count":1,"kind":"tool"},{"name":"wheat","count":12,"kind":"food"}]}"#);
        assert!(errors(ui).is_empty(), "no errors");
        assert_eq!(probe(ui, "document.querySelector('#title').textContent"), "\"Turn 3 · spring\"");
        assert_eq!(probe(ui, "document.querySelector('#gold').textContent + '|' + document.querySelector('#gold').className"), "\"25|cell\"");
        assert_eq!(probe(ui, "document.querySelector('#hp').style.width"), "\"50%\"");
        assert_eq!(probe(ui, "document.querySelector('#icon').getAttribute('alt')"), "\"Ada\"");
        assert_eq!(probe(ui, "document.querySelector('#warn').style.display"), "\"none\"");
        assert_eq!(probe(ui, "[...document.querySelectorAll('#items li')].map(l => l.textContent + ':' + l.className).join(',')"), "\"axe ×1:tool,wheat ×12:food\"");

        // An update: values change in place, the list shrinks and grows.
        model("player", r#"{"gold":4,"hp":0.25,"name":"Ada","items":[{"name":"axe","count":2,"kind":"tool"}]}"#);
        assert_eq!(probe(ui, "document.querySelector('#gold').className"), "\"cell low\"");
        assert_eq!(probe(ui, "document.querySelector('#warn').style.display"), "\"\"");
        assert_eq!(probe(ui, "[...document.querySelectorAll('#items li')].map(l => l.textContent).join(',')"), "\"axe ×2\"");
        model("player", r#"{"gold":4,"hp":0.25,"name":"Ada","items":[{"name":"axe","count":2,"kind":"tool"},{"name":"iron","count":5,"kind":"ore"},{"name":"fish","count":3,"kind":"food"}]}"#);
        assert_eq!(probe(ui, "[...document.querySelectorAll('#items li')].map(l => l.textContent + ':' + l.className).join(',')"), "\"axe ×2:tool,iron ×5:ore,fish ×3:food\"");

        // A bound click runs with its item in scope.
        click(ui, "#items li:nth-child(3)");
        assert!(events(ui).contains(&("pick".into(), "[2,\"fish\"]".into())), "data-bind-click");
        assert!(errors(ui).is_empty(), "no errors");

        // The cost of a sync of this page with an unchanged model, and a changed one.
        let t = std::time::Instant::now();
        for i in 0..200 { model("player", &format!(r#"{{"gold":{},"hp":0.25,"name":"Ada","items":[{{"name":"axe","count":2,"kind":"tool"}}]}}"#, i % 2)); }
        println!("a model update and sync: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / 200.0);
        vitrail_destroy(ui);
    }
}
