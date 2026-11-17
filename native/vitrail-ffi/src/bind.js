// Data binding, as Coherent's: the game hands the page models (plain objects), and elements show them through
// `data-bind-*` attributes with {{expressions}}, no script needed:
//   data-bind-value="{{player.gold}}"            the text (any text around expressions is kept: "{{a}} / {{b}}")
//   data-bind-html="{{note.html}}"               inner HTML
//   data-bind-if="{{player.gold > 0}}"           shown or display: none
//   data-bind-class="{{unit.kind}}"              classes added (and taken off when the value changes)
//   data-bind-class-toggle="low:{{s.wood < 10}};full:{{s.full}}"
//   data-bind-style-width="{{hp * 100}}%"        any CSS property
//   data-bind-attr-src="icons/{{item.icon}}.png" any attribute
//   data-bind-for="item:{{player.items}}"        the element repeated per item ("i, item:{{…}}" names the index too)
//   data-bind-click="engine.trigger('sell', item.id)"   an event (click, mouseenter, …) runs the code, `event` in scope
// Bindings are found once and again only after the document changes; each sync writes only values that changed.
(() => {
  'use strict';
  const G = globalThis;
  const models = Object.create(null);
  const compiled = new Map();
  let bindings = [];
  let repeats = [];
  let seen = -1;

  // An expression in the models' scope (and a loop's names); sloppy mode for `with`.
  function compile(code, statement) {
    const key = (statement ? 's:' : 'e:') + code;
    let f = compiled.get(key);
    if (!f) {
      try { f = new Function('$s', 'event', statement ? `with ($s) { ${code} }` : `with ($s) { return (${code}); }`); }
      catch (err) { reportError(new SyntaxError(`data-bind: ${code}: ${err.message}`)); f = () => undefined; }
      compiled.set(key, f);
    }
    return f;
  }
  // "text {{a}} more {{b}}" → a function of the scope: the value itself for a lone {{a}}, else the joined text.
  function template(text) {
    const parts = text.split(/\{\{([\s\S]+?)\}\}/);
    if (parts.length === 3 && parts[0] === '' && parts[2] === '') { const f = compile(parts[1]); return (s) => f(s); }
    const fs = parts.map((p, i) => i % 2 ? compile(p) : null);
    return (s) => parts.map((p, i) => { if (!(i % 2)) return p; const v = fs[i](s); return v == null ? '' : v; }).join('');
  }
  // A binding to a model that hasn't come yet (a ReferenceError) waits quietly, as in Coherent; other errors are reported.
  const run = (f, scope) => { try { return f(scope); } catch (err) { if (!(err instanceof ReferenceError)) reportError(err); return undefined; } };

  // The bindings of one element: [{ kind, name, f, last }].
  function collect(el, out) {
    for (const name of el.getAttributeNames()) {
      if (!name.startsWith('data-bind-') || name === 'data-bind-for') continue;
      const value = el.getAttribute(name);
      const kind = name.slice(10);
      if (kind === 'class-toggle') {
        for (const pair of value.split(';')) { const i = pair.indexOf(':'); if (i > 0) out.push({ el, kind, name: pair.slice(0, i).trim(), f: template(pair.slice(i + 1).trim()), last: undefined }); }
      } else if (['click', 'dblclick', 'mousedown', 'mouseup', 'mouseenter', 'mouseleave', 'mouseover', 'mouseout', 'pointerdown', 'pointerup', 'input', 'change', 'wheel'].includes(kind)) {
        out.push({ el, kind: 'event', name: kind, f: compile(value, true), last: undefined });
      } else {
        out.push({ el, kind, name: kind.startsWith('style-') ? kind.slice(6) : kind.startsWith('attr-') ? kind.slice(5) : kind, f: template(value), last: undefined });
      }
    }
  }
  // Everything under `root` (not inside a repeated element's template), and the repeats as their own entries.
  function scan(root, out, top) {
    const walk = (el) => {
      if (el._bindCopy) return;
      if (el.hasAttribute('data-bind-for')) { const r = repeat(el); out.push(r); if (top) repeats.push(r); return; }
      collect(el, out);
      for (const c of el.children) walk(c);
    };
    for (const c of root.children) walk(c);
  }
  function repeat(el) {
    const spec = el.getAttribute('data-bind-for');
    const i = spec.indexOf(':');
    const names = spec.slice(0, i).split(',').map((s) => s.trim());
    const [index, item] = names.length > 1 ? names : [null, names[0]];
    const anchor = G.document.createComment('data-bind-for');
    el.parentNode.insertBefore(anchor, el);
    el.remove();
    el.removeAttribute('data-bind-for');
    return { kind: 'for', el, anchor, index, item, f: template(spec.slice(i + 1).trim()), copies: [] };
  }

  function apply(b, scope) {
    if (b.kind === 'event') {
      if (!b.wired) { b.wired = true; b.el.addEventListener(b.name, (e) => { try { b.f(b.scope, e); } catch (err) { reportError(err); } }); }
      b.scope = scope;
      return;
    }
    if (b.kind === 'for') return applyFor(b, scope);
    const v = run(b.f, scope);
    if (v === b.last && b.kind !== 'class-toggle') return;
    const el = b.el;
    switch (b.kind) {
      case 'value': el.textContent = v == null ? '' : String(v); break;
      case 'html': el.innerHTML = v == null ? '' : String(v); break;
      case 'if': el.style.display = v ? '' : 'none'; break;
      case 'class': {
        const old = b.last ? String(b.last).split(/\s+/).filter((x) => x) : [];
        const now = v ? String(v).split(/\s+/).filter((x) => x) : [];
        el.classList.remove(...old.filter((c) => !now.includes(c)));
        el.classList.add(...now.filter((c) => !old.includes(c)));
        break;
      }
      case 'class-toggle': if (!!v !== b.last) el.classList.toggle(b.name, !!v); b.last = !!v; return;
      default:
        if (b.kind.startsWith('style-')) el.style.setProperty(b.name, v == null ? '' : String(v));
        else if (b.kind.startsWith('attr-')) v == null || v === false ? el.removeAttribute(b.name) : el.setAttribute(b.name, v === true ? '' : String(v));
        else reportError(new Error(`data-bind: unknown binding data-bind-${b.kind}`));
    }
    b.last = v;
  }

  // A repeated element: as many copies as items (added or taken off at the end), each bound with its item.
  function applyFor(b, scope) {
    const list = run(b.f, scope);
    const items = Array.isArray(list) ? list : list && typeof list === 'object' ? Object.values(list) : [];
    while (b.copies.length > items.length) b.copies.pop().el.remove();
    while (b.copies.length < items.length) {
      const el = b.el.cloneNode(true);
      el._bindCopy = true;
      const before = b.copies.length ? b.copies[b.copies.length - 1].el.nextSibling : b.anchor.nextSibling;
      b.anchor.parentNode.insertBefore(el, before);
      const inner = [];
      collect(el, inner);
      scan(el, inner);
      b.copies.push({ el, bindings: inner });
    }
    items.forEach((it, i) => {
      const s = Object.create(scope);
      s[b.item] = it;
      if (b.index) s[b.index] = i;
      for (const x of b.copies[i].bindings) apply(x, s);
    });
  }

  function sync() {
    const doc = G.document;
    if (!doc.body) return;
    if (seen !== G.__domVersion) {
      // Found again after the page changed; repeats found earlier keep their copies (their templates are out of the DOM).
      repeats = repeats.filter((r) => r.anchor.isConnected);
      bindings = repeats.slice();
      scan(doc.documentElement, bindings, true);
    }
    for (const b of bindings) apply(b, models);
    seen = G.__domVersion;
  }

  Object.assign(G.engine, {
    models,
    createModel(name, model) { models[name] = model; sync(); },
    updateWholeModel() {}, // the next synchronizeModels() writes what changed
    synchronizeModels: sync,
    removeModel(name) { delete models[name]; sync(); },
  });
  // From the game (WebPanel.SetModel): a whole model as JSON, then a sync.
  G.engine.on('__model', (name, model) => { models[name] = model; sync(); });
})();
