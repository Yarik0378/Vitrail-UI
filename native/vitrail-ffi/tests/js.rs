//! JavaScript in the page: scripts run, change the DOM, hear clicks and timers, and talk to the game.
use std::ffi::{CString, c_char};
use vitrail::*;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

unsafe fn logs(ui: *mut Vitrail) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut level = 0i32;
    while unsafe { vitrail_js_log(ui, buf.as_mut_ptr() as *mut c_char, buf.len(), &mut level) } == 1 {
        let n = buf.iter().position(|&b| b == 0).unwrap();
        out.push((level, String::from_utf8_lossy(&buf[..n]).into_owned()));
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

const PAGE: &str = r#"<!doctype html><html><body>
<div id="box" style="position:absolute;left:10px;top:10px;width:100px;height:40px">old</div>
<ul id="list"></ul>
<script>
  console.log('hello', 1, { a: 2 });
  const box = document.getElementById('box');
  box.textContent = 'new';
  box.classList.add('on', 'big');
  box.style.backgroundColor = 'red';
  for (const name of ['a', 'b', 'c']) {
    const li = document.createElement('li');
    li.textContent = name;
    document.querySelector('#list').appendChild(li);
  }
  box.addEventListener('click', (e) => engine.trigger('clicked', e.target.id, Math.round(e.clientX)));
  document.body.addEventListener('click', () => engine.trigger('bubbled'));
  setTimeout(() => { box.textContent = 'later'; engine.trigger('timer', box.textContent); }, 500);
  engine.on('set', (v) => { box.textContent = v; engine.trigger('got', box.textContent); });
  Promise.resolve().then(() => engine.trigger('micro'));
  document.addEventListener('DOMContentLoaded', () => engine.trigger('ready', document.querySelectorAll('li').length, box.className));
  undefinedFunction();
</script>
</body></html>"#;

#[test]
fn scripts_change_the_page_and_talk_to_the_game() {
    unsafe {
        let ui = vitrail_create(s(PAGE).as_ptr(), s("file:///tmp/js-test.html").as_ptr(), 400, 200, 2.0);
        let log = logs(ui);
        println!("log: {log:?}");
        assert!(log.iter().any(|(l, m)| *l == 0 && m == "hello 1 {\"a\":2}"), "console.log");
        assert!(log.iter().any(|(l, m)| *l == 2 && m.contains("undefinedFunction")), "an uncaught error is reported");
        let ev = events(ui);
        println!("events: {ev:?}");
        assert!(ev.contains(&("micro".into(), "[]".into())), "promise jobs run");
        assert!(ev.contains(&("ready".into(), "[3,\"on big\"]".into())), "DOM built, classes set, DOMContentLoaded fired");

        // A click on the box (device px 2×) reaches its listener and bubbles to the body.
        vitrail_pointer_move(ui, 60.0, 50.0);
        vitrail_pointer_down(ui, 60.0, 50.0);
        let (mut a, mut b) = ([0 as c_char; 64], [0 as c_char; 64]);
        vitrail_pointer_up(ui, 60.0, 50.0, a.as_mut_ptr(), b.as_mut_ptr(), 64);
        let ev = events(ui);
        println!("click: {ev:?}");
        assert_eq!(ev, vec![("clicked".into(), "[\"box\",30]".into()), ("bubbled".into(), "[]".into())]);

        // The timer runs on the page clock.
        vitrail_tick(ui, 0.2);
        assert!(events(ui).is_empty(), "not yet at 200 ms");
        vitrail_tick(ui, 0.6);
        assert_eq!(events(ui), vec![("timer".into(), "[\"later\"]".into())]);

        // The game calls the page.
        assert_eq!(vitrail_js_call(ui, s("set").as_ptr(), s("[\"from C#\"]").as_ptr()), 1);
        assert_eq!(events(ui), vec![("got".into(), "[\"from C#\"]".into())]);
        assert_eq!(vitrail_needs_paint(ui), 1, "a change from a script marks the page for painting");
        vitrail_destroy(ui);
    }
}

#[test]
fn a_page_without_scripts_gets_no_engine() {
    unsafe {
        let ui = vitrail_create(s("<div>plain</div>").as_ptr(), s("").as_ptr(), 100, 100, 1.0);
        assert_eq!(vitrail_js_call(ui, s("x").as_ptr(), s("[]").as_ptr()), 0);
        vitrail_destroy(ui);
    }
}

#[test]
fn animation_and_transition_events_reach_scripts() {
    unsafe {
        let page = r#"<style>
          #t { width: 50px; height: 20px; opacity: 1; transition: opacity .3s linear; }
          #t.off { opacity: 0; }
          #a { width: 50px; height: 20px; animation: pulse .2s linear 2; }
          @keyframes pulse { from { transform: scale(1) } to { transform: scale(1.2) } }
        </style><div id="t"></div><div id="a"></div><script>
          const log = (e) => engine.trigger(e.type, e.target.id, e.propertyName ?? e.animationName);
          for (const k of ['transitionrun', 'transitionstart', 'transitionend', 'animationstart', 'animationiteration', 'animationend']) document.body.addEventListener(k, log);
          engine.on('go', () => document.getElementById('t').classList.add('off'));
        </script>"#;
        let ui = vitrail_create(s(page).as_ptr(), s("").as_ptr(), 200, 100, 1.0);
        let mut r = [0f32; 4];
        let mut frame = |t: f64| { vitrail_tick(ui, t); vitrail_rect(ui, s("#t").as_ptr(), r.as_mut_ptr()); };
        frame(0.0);
        vitrail_js_call(ui, s("go").as_ptr(), s("[]").as_ptr());
        for i in 1..=40 { frame(i as f64 * 0.025); }
        let got: Vec<String> = events(ui).into_iter().map(|(n, j)| format!("{n} {j}")).collect();
        println!("{got:#?}");
        let want = [
            "animationstart [\"a\",\"pulse\"]", "animationiteration [\"a\",\"pulse\"]", "animationend [\"a\",\"pulse\"]",
            "transitionrun [\"t\",\"opacity\"]", "transitionstart [\"t\",\"opacity\"]", "transitionend [\"t\",\"opacity\"]",
        ];
        for w in want { assert!(got.iter().any(|g| g == w), "missing {w}"); }
        let pos = |w: &str| got.iter().position(|g| g == w).unwrap();
        assert!(pos(want[0]) < pos(want[1]) && pos(want[1]) < pos(want[2]), "animation events in order");
        assert!(pos(want[3]) < pos(want[5]), "transition events in order");
        assert_eq!(got.iter().filter(|g| g.starts_with("animationiteration")).count(), 1, "two iterations: one iteration event");
        vitrail_destroy(ui);
    }
}

#[test]
fn element_animate_plays_and_finishes() {
    unsafe {
        let page = r#"<div id="a" style="width:50px;height:20px"></div><div id="b" style="width:50px;height:20px"></div><script>
          const a = document.getElementById('a');
          const fade = a.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 300 });
          const move = a.animate({ transform: ['translateX(0)', 'translateX(40px)'] }, { duration: 200, fill: 'forwards' });
          fade.finished.then(() => engine.trigger('fade', fade.playState));
          move.onfinish = () => engine.trigger('move', move.playState, a.getAnimations().length);
          const spin = document.getElementById('b').animate([{ transform: 'rotate(0)' }, { transform: 'rotate(1turn)' }], { duration: 1000, iterations: Infinity });
          spin.finished.catch((e) => engine.trigger('spin', e.name, spin.playState));
          engine.on('stop', () => spin.cancel());
        </script>"#;
        let ui = vitrail_create(s(page).as_ptr(), s("").as_ptr(), 200, 100, 1.0);
        let mut r = [0f32; 4];
        let mut animating = 0;
        for i in 0..=30 {
            if vitrail_tick(ui, i as f64 * 0.02) != 0 { animating += 1; }
            vitrail_rect(ui, s("#a").as_ptr(), r.as_mut_ptr());
        }
        assert!(animating > 10, "the animations play ({animating} animating frames)");
        vitrail_js_call(ui, s("stop").as_ptr(), s("[]").as_ptr());
        for i in 31..=40 { vitrail_tick(ui, i as f64 * 0.02); vitrail_rect(ui, s("#a").as_ptr(), r.as_mut_ptr()); }
        let got = events(ui);
        println!("{got:?}");
        assert_eq!(got.iter().filter(|e| e.0 == "move").count(), 1, "onfinish once");
        assert!(got.contains(&("move".into(), "[\"finished\",2]".into())), "fill: forwards stays among the element's animations (fade still runs)");
        assert!(got.contains(&("fade".into(), "[\"finished\"]".into())), "finished resolves");
        assert!(got.contains(&("spin".into(), "[\"AbortError\",\"idle\"]".into())), "cancel rejects finished");
        assert_eq!(vitrail_tick(ui, 1.0), 0, "nothing animates after the cancel (the forwards fill holds still)");
        vitrail_destroy(ui);
    }
}

unsafe fn key(ui: *mut Vitrail, k: &str, code: &str) -> i32 {
    unsafe {
        let r = vitrail_key(ui, 1, s(k).as_ptr(), s(code).as_ptr(), 0, 0);
        vitrail_key(ui, 0, s(k).as_ptr(), s(code).as_ptr(), 0, 0);
        r
    }
}

#[test]
fn keyboard_focus_and_text_fields() {
    unsafe {
        let page = r#"<input id="name" style="position:absolute;left:10px;top:10px;width:150px;height:24px" value="Al">
        <div id="x" style="position:absolute;left:10px;top:60px;width:50px;height:20px"></div><script>
          document.addEventListener('keydown', (e) => { engine.trigger('key', e.key, e.code, e.target.localName); if (e.key === 'F1') e.preventDefault(); });
          const f = document.getElementById('name');
          f.addEventListener('focus', () => engine.trigger('focus'));
          f.addEventListener('input', () => engine.trigger('input', f.value));
        </script>"#;
        let ui = vitrail_create(s(page).as_ptr(), s("").as_ptr(), 400, 200, 2.0);
        // Nothing focused: keys reach the body's listeners, the game keeps them.
        assert_eq!(key(ui, "h", "KeyH"), 0, "a key with no field focused stays the game's");
        assert_eq!(events(ui), vec![("key".into(), "[\"h\",\"KeyH\",\"body\"]".into())]);
        assert_eq!(key(ui, "F1", "F1"), 1, "a prevented key is the page's");
        events(ui);
        // A press on the field focuses it; typing edits it and fires input.
        vitrail_pointer_down(ui, 60.0, 40.0);
        let (mut a, mut b) = ([0 as c_char; 64], [0 as c_char; 64]);
        vitrail_pointer_up(ui, 60.0, 40.0, a.as_mut_ptr(), b.as_mut_ptr(), 64);
        assert!(events(ui).contains(&("focus".into(), "[]".into())), "focus on press");
        assert_eq!(key(ui, "End", "End"), 1, "a focused field takes keys");
        for (k, c) in [("i", "KeyI"), ("c", "KeyC"), ("e", "KeyE")] { key(ui, k, c); }
        let ev = events(ui);
        println!("{ev:?}");
        assert!(ev.contains(&("input".into(), "[\"Alice\"]".into())), "typed into the field: {ev:?}");
        key(ui, "Backspace", "Backspace");
        let ev = events(ui);
        assert!(ev.contains(&("input".into(), "[\"Alic\"]".into())), "Backspace: {ev:?}");
        assert_eq!(vitrail_needs_paint(ui), 1);
        // A press elsewhere takes the focus away: keys are the game's again.
        vitrail_pointer_down(ui, 60.0, 140.0);
        vitrail_pointer_up(ui, 60.0, 140.0, a.as_mut_ptr(), b.as_mut_ptr(), 64);
        assert_eq!(key(ui, "j", "KeyJ"), 0);
        vitrail_destroy(ui);
    }
}

#[test]
fn fetch_mutation_observer_and_inner_html() {
    unsafe {
        let page = r#"<ul id="l"><li class="a">one</li></ul><script>
          fetch('data/player.json').then((r) => r.json()).then((p) => engine.trigger('fetched', p.gold, p.name));
          fetch('data/missing.json').then((r) => engine.trigger('missing', r.ok, r.status));
          fetch('https://example.com/x').catch((e) => engine.trigger('remote', e.name));
          const l = document.getElementById('l');
          new MutationObserver((recs) => engine.trigger('mutations', recs.map((r) => r.type + ':' + (r.attributeName || r.addedNodes.length + '/' + r.removedNodes.length)).join(','))).observe(l, { childList: true, subtree: true, attributes: true, attributeOldValue: true });
          const li = document.createElement('li'); li.textContent = 'two & <three>'; l.appendChild(li);
          l.firstElementChild.classList.add('b');
          l.firstElementChild.remove();
          engine.trigger('html', l.innerHTML, l.outerHTML.length);
        </script>"#;
        let base = format!("file://{}/../../tests/pages/x.html", env!("CARGO_MANIFEST_DIR"));
        let ui = vitrail_create(s(page).as_ptr(), s(&base).as_ptr(), 400, 200, 1.0);
        let ev = events(ui);
        println!("{ev:?}");
        assert!(ev.contains(&("fetched".into(), "[7,\"Ada\"]".into())), "fetch a JSON file next to the page");
        assert!(ev.contains(&("missing".into(), "[false,404]".into())));
        assert!(ev.contains(&("remote".into(), "[\"TypeError\"]".into())));
        assert!(ev.contains(&("mutations".into(), "[\"childList:1/0,attributes:class,childList:0/1\"]".into())), "MutationObserver records in one batch");
        assert!(ev.contains(&("html".into(), "[\"<li>two &amp; &lt;three&gt;</li>\",48]".into())), "innerHTML and outerHTML");
        vitrail_destroy(ui);
    }
}

#[test]
fn a_field_made_by_a_script_takes_typing() {
    unsafe {
        let page = r#"<body><script>
          const f = document.createElement('input'); f.value = 'Ag'; document.body.appendChild(f); f.focus();
          f.addEventListener('input', () => engine.trigger('typed', f.value));
        </script></body>"#;
        let ui = vitrail_create(s(page).as_ptr(), s("").as_ptr(), 400, 200, 1.0);
        // Made again after load, typed into at once (no paint between).
        vitrail_js_eval(ui, s("document.querySelector('input').remove(); const g = document.createElement('input'); g.value = 'Ag'; document.body.appendChild(g); g.focus(); g.addEventListener('input', () => engine.trigger('typed', g.value));").as_ptr());
        assert_eq!(vitrail_editing(ui), 1, "the script's field is focused and editable");
        key(ui, "End", "End");
        key(ui, "e", "KeyE");
        assert!(events(ui).contains(&("typed".into(), "[\"Age\"]".into())));
        vitrail_destroy(ui);
    }
}

unsafe fn press(ui: *mut Vitrail, x0: f32, x1: f32, y: f32) {
    unsafe {
        vitrail_pointer_move(ui, x0, y);
        vitrail_pointer_down(ui, x0, y);
        vitrail_pointer_move(ui, x1, y);
        let (mut a, mut b) = ([0 as c_char; 64], [0 as c_char; 64]);
        vitrail_pointer_up(ui, x1, y, a.as_mut_ptr(), b.as_mut_ptr(), 64);
    }
}

unsafe fn value(ui: *mut Vitrail) -> String {
    unsafe {
        events(ui);
        vitrail_js_eval(ui, s("engine.trigger('v', document.getElementById('f').value)").as_ptr());
        let e = events(ui);
        let v = &e.iter().find(|(n, _)| n == "v").unwrap().1;
        v.trim_start_matches("[\"").trim_end_matches("\"]").to_string()
    }
}

#[test]
fn text_fields_with_the_mouse_clipboard_and_ime() {
    unsafe {
        let page = r#"<input id="f" style="position:absolute;left:0;top:0;width:300px;height:30px;font:20px monospace;padding:0;border:0" value="hello world"><script>document.body.addEventListener('keydown',()=>{});</script>"#;
        let ui = vitrail_create(s(page).as_ptr(), s("").as_ptr(), 400, 100, 1.0);
        // A press between "hello" and " world" puts the caret there.
        let mut r = [0f32; 4];
        vitrail_rect(ui, s("#f").as_ptr(), r.as_mut_ptr());
        let ch = 12.0f32; // monospace 20px: about 12 px a character
        press(ui, 5.0 * ch, 5.0 * ch, 15.0);
        key(ui, "X", "KeyX");
        assert_eq!(value(ui), "helloX world", "the caret where the press was");
        // A drag selects; typing replaces the selection.
        press(ui, 7.0 * ch, 12.0 * ch + 2.0, 15.0);
        key(ui, "Y", "KeyY");
        assert_eq!(value(ui), "helloX Y", "the dragged selection replaced");
        // Select all and copy: the game gets the text.
        let meta = if cfg!(target_os = "macos") { 8 } else { 2 };
        vitrail_key(ui, 1, s("a").as_ptr(), s("KeyA").as_ptr(), meta, 0);
        vitrail_key(ui, 1, s("c").as_ptr(), s("KeyC").as_ptr(), meta, 0);
        let mut buf = [0 as c_char; 256];
        assert_eq!(vitrail_clipboard_take(ui, buf.as_mut_ptr(), 256), 1, "copied");
        assert_eq!(std::ffi::CStr::from_ptr(buf.as_ptr()).to_str().unwrap(), "helloX Y");
        // Paste: the page asks, the game answers with the system clipboard's text.
        key(ui, "End", "End");
        vitrail_key(ui, 1, s("v").as_ptr(), s("KeyV").as_ptr(), meta, 0);
        assert_eq!(vitrail_paste_wanted(ui), 1, "the page asked to paste");
        vitrail_paste(ui, s("!").as_ptr());
        assert_eq!(value(ui), "helloX Y!", "pasted at the caret");
        // An IME: the composition shows, then commits.
        let mut area = [0f32; 4];
        assert_eq!(vitrail_ime_state(ui, area.as_mut_ptr()), 1, "a focused field wants the IME");
        vitrail_ime(ui, 0, s("ni").as_ptr(), 2);
        vitrail_ime(ui, 1, s("你").as_ptr(), -1);
        assert_eq!(value(ui), "helloX Y!你", "the committed composition");
        vitrail_destroy(ui);
    }
}

#[test]
fn inner_html_setter_builds_children() {
    unsafe {
        let page = r#"<div id="row"></div><script>
          const r = document.getElementById('row');
          r.innerHTML = '<div class="tool">A</div><div class="tool"><span>B</span></div>';
          engine.trigger('kids', r.children.length, document.querySelectorAll('.tool').length, r.innerHTML);
        </script>"#;
        let base = format!("file://{}/../../tests/pages/x.html", env!("CARGO_MANIFEST_DIR"));
        let ui = vitrail_create(s(page).as_ptr(), s(&base).as_ptr(), 400, 200, 1.0);
        let ev = events(ui);
        println!("{ev:?}");
        assert!(ev.contains(&("kids".into(), "[2,2,\"<div class=\\\"tool\\\">A</div><div class=\\\"tool\\\"><span>B</span></div>\"]".into())), "innerHTML = … makes the children");
        vitrail_destroy(ui);
    }
}
