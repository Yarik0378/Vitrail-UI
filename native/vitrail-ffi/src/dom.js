// The page's DOM, events, timers, console and the game's bridge, over the native node operations (__n, js.rs).
// Node objects are made once per node id and kept, so the same node is always the same object.
(() => {
  'use strict';
  const N = __n;
  const G = globalThis;
  const nodes = new Map();
  // Goes up with every change to the tree's shape (bind.js finds its bindings again then).
  G.__domVersion = 0;
  const bump = () => { G.__domVersion++; };

  // ---------- MutationObserver: the mutation points below report here (nothing happens while none observes) ----------
  const observing = [];
  function record(r) {
    if (!observing.length) return;
    const t = r.target;
    for (const o of observing) {
      if (o.target !== t && !(o.opts.subtree && o.target.contains(t))) continue;
      if (r.type === 'attributes' && (!o.opts.attributes || o.opts.attributeFilter && !o.opts.attributeFilter.includes(r.attributeName))) continue;
      if (r.type === 'childList' && !o.opts.childList) continue;
      if (r.type === 'characterData' && !o.opts.characterData) continue;
      const keepOld = r.type === 'attributes' ? o.opts.attributeOldValue : o.opts.characterDataOldValue;
      o.mo._q.push({ type: r.type, target: t, addedNodes: r.addedNodes || [], removedNodes: r.removedNodes || [], previousSibling: null, nextSibling: null,
        attributeName: r.attributeName ?? null, attributeNamespace: null, oldValue: keepOld ? (r.oldValue ?? null) : null });
      if (!o.mo._queued) { o.mo._queued = true; Promise.resolve().then(() => o.mo._deliver()); }
    }
  }
  class MutationObserver {
    constructor(cb) { this._cb = cb; this._q = []; this._queued = false; }
    observe(target, opts = {}) {
      opts = { ...opts };
      if (opts.attributeOldValue || opts.attributeFilter) opts.attributes ??= true;
      if (opts.characterDataOldValue) opts.characterData ??= true;
      const i = observing.findIndex((o) => o.mo === this && o.target === target);
      if (i >= 0) observing[i].opts = opts; else observing.push({ mo: this, target, opts });
    }
    disconnect() { for (let i = observing.length - 1; i >= 0; i--) if (observing[i].mo === this) observing.splice(i, 1); this._q = []; }
    takeRecords() { const q = this._q; this._q = []; return q; }
    _deliver() { this._queued = false; const q = this.takeRecords(); if (q.length) { try { this._cb(q, this); } catch (err) { reportError(err); } } }
  }
  const setA = (id, name, v) => { const old = observing.length ? N.attr(id, name) : null; N.setAttr(id, name, v); if (observing.length) record({ type: 'attributes', target: wrap(id), attributeName: name, oldValue: old }); };
  const rmA = (id, name) => { const old = observing.length ? N.attr(id, name) : null; N.rmAttr(id, name); if (observing.length && old != null) record({ type: 'attributes', target: wrap(id), attributeName: name, oldValue: old }); };

  const kebab = (s) => s.startsWith('--') ? s : s.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase());
  const camel = (s) => s.replace(/-([a-z])/g, (_, c) => c.toUpperCase());

  function wrap(id) {
    if (!id) return null;
    let n = nodes.get(id);
    if (n) return n;
    const t = N.type(id);
    if (t === 0) return null;
    n = t === 1 ? new Element(id) : t === 3 ? new Text(id) : t === 9 ? new Document(id) : new Comment(id);
    nodes.set(id, n);
    return n;
  }
  const list = (ids) => ids.map(wrap).filter((x) => x);

  // ---------- Events ----------
  class Event {
    constructor(type, init = {}) {
      this.type = type; this.bubbles = !!init.bubbles; this.cancelable = !!init.cancelable; this.composed = !!init.composed;
      this.target = null; this.currentTarget = null; this.eventPhase = 0; this.defaultPrevented = false;
      this.timeStamp = N.now(); this.isTrusted = false; this._stop = false; this._stopNow = false;
    }
    stopPropagation() { this._stop = true; }
    stopImmediatePropagation() { this._stop = true; this._stopNow = true; }
    preventDefault() { if (this.cancelable) this.defaultPrevented = true; }
    composedPath() { return this._path || []; }
  }
  Event.NONE = 0; Event.CAPTURING_PHASE = 1; Event.AT_TARGET = 2; Event.BUBBLING_PHASE = 3;
  class CustomEvent extends Event { constructor(type, init = {}) { super(type, init); this.detail = init.detail ?? null; } }
  class UIEvent extends Event { constructor(type, init = {}) { super(type, init); this.detail = init.detail ?? 0; this.view = G; } }
  class MouseEvent extends UIEvent {
    constructor(type, init = {}) {
      super(type, init);
      for (const k of ['clientX', 'clientY', 'screenX', 'screenY', 'button', 'buttons', 'movementX', 'movementY']) this[k] = init[k] ?? 0;
      this.pageX = this.clientX; this.pageY = this.clientY; this.offsetX = init.offsetX ?? 0; this.offsetY = init.offsetY ?? 0;
      this.ctrlKey = !!init.ctrlKey; this.shiftKey = !!init.shiftKey; this.altKey = !!init.altKey; this.metaKey = !!init.metaKey;
      this.relatedTarget = init.relatedTarget ?? null;
    }
  }
  class PointerEvent extends MouseEvent { constructor(type, init = {}) { super(type, init); this.pointerId = 1; this.pointerType = 'mouse'; this.isPrimary = true; } }
  class WheelEvent extends MouseEvent { constructor(type, init = {}) { super(type, init); this.deltaX = init.deltaX ?? 0; this.deltaY = init.deltaY ?? 0; this.deltaZ = 0; this.deltaMode = 0; } }
  class KeyboardEvent extends UIEvent {
    constructor(type, init = {}) {
      super(type, init); this.key = init.key ?? ''; this.code = init.code ?? ''; this.repeat = !!init.repeat;
      this.ctrlKey = !!init.ctrlKey; this.shiftKey = !!init.shiftKey; this.altKey = !!init.altKey; this.metaKey = !!init.metaKey;
    }
  }
  class FocusEvent extends UIEvent {}
  class TransitionEvent extends Event { constructor(type, init = {}) { super(type, init); this.propertyName = init.propertyName ?? ''; this.elapsedTime = init.elapsedTime ?? 0; this.pseudoElement = init.pseudoElement ?? ''; } }
  class AnimationEvent extends Event { constructor(type, init = {}) { super(type, init); this.animationName = init.animationName ?? ''; this.elapsedTime = init.elapsedTime ?? 0; this.pseudoElement = init.pseudoElement ?? ''; } }
  class DataTransfer {
    constructor() { this._d = new Map(); this.dropEffect = 'none'; this.effectAllowed = 'all'; this.files = []; this.items = []; this._image = null; }
    get types() { return [...this._d.keys()]; }
    setData(t, v) { this._d.set(String(t).toLowerCase(), String(v)); }
    getData(t) { return this._d.get(String(t).toLowerCase()) ?? ''; }
    clearData(t) { if (t === undefined) this._d.clear(); else this._d.delete(String(t).toLowerCase()); }
    setDragImage(el, x, y) { this._image = { el, x, y }; }
  }
  class DragEvent extends MouseEvent { constructor(type, init = {}) { super(type, init); this.dataTransfer = init.dataTransfer ?? null; } }
  // Drag and drop needs the mouse's events: asking for any drag event asks for them too.
  const DRAG = new Set(['dragstart', 'drag', 'dragenter', 'dragover', 'dragleave', 'drop', 'dragend']);
  const listen = (type) => { N.listen(type); if (DRAG.has(type)) for (const t of ['mousedown', 'mousemove', 'mouseup', 'click']) N.listen(t); };
  class InputEvent extends UIEvent { constructor(type, init = {}) { super(type, init); this.data = init.data ?? null; this.inputType = init.inputType ?? ''; } }

  class EventTarget {
    addEventListener(type, fn, opt) {
      if (!fn) return;
      const capture = typeof opt === 'boolean' ? opt : !!opt?.capture;
      const once = !!opt?.once;
      const l = (this._l ??= new Map());
      let a = l.get(type);
      if (!a) { a = []; l.set(type, a); listen(type); }
      if (!a.some((x) => x.fn === fn && x.capture === capture)) a.push({ fn, capture, once });
      if (opt?.signal) opt.signal.addEventListener('abort', () => this.removeEventListener(type, fn, opt));
    }
    removeEventListener(type, fn, opt) {
      const capture = typeof opt === 'boolean' ? opt : !!opt?.capture;
      const a = this._l?.get(type);
      if (!a) return;
      const i = a.findIndex((x) => x.fn === fn && x.capture === capture);
      if (i >= 0) a.splice(i, 1);
    }
    _fire(e, phase) {
      const a = this._l?.get(e.type);
      if (a) {
        e.currentTarget = this; e.eventPhase = phase;
        for (const x of a.slice()) {
          if (phase === 1 && !x.capture || phase === 3 && x.capture) continue;
          if (x.once) this.removeEventListener(e.type, x.fn, x.capture);
          try { typeof x.fn === 'function' ? x.fn.call(this, e) : x.fn.handleEvent(e); } catch (err) { reportError(err); }
          if (e._stopNow) break;
        }
      }
      const on = this['on' + e.type];
      if (phase !== 1 && typeof on === 'function' && !e._stopNow) { try { on.call(this, e); } catch (err) { reportError(err); } }
    }
    dispatchEvent(e) {
      e.target = this;
      const path = [];
      for (let n = this; n; n = n === G.document ? G : n.parentNode) { path.push(n); if (n === G) break; }
      e._path = path;
      for (let i = path.length - 1; i > 0 && !e._stop; i--) path[i]._fire(e, 1);
      if (!e._stop) this._fire(e, 2);
      if (e.bubbles) for (let i = 1; i < path.length && !e._stop; i++) path[i]._fire(e, 3);
      e.currentTarget = null; e.eventPhase = 0;
      return !e.defaultPrevented;
    }
  }

  // ---------- Nodes ----------
  class Node extends EventTarget {
    constructor(id) { super(); this._id = id; }
    get nodeType() { return N.type(this._id); }
    get ownerDocument() { return G.document; }
    get parentNode() { return wrap(N.parent(this._id)); }
    get parentElement() { const p = this.parentNode; return p && p.nodeType === 1 ? p : null; }
    get childNodes() { return list(N.children(this._id)); }
    get firstChild() { const c = N.children(this._id); return c.length ? wrap(c[0]) : null; }
    get lastChild() { const c = N.children(this._id); return c.length ? wrap(c[c.length - 1]) : null; }
    _sib(d) { const p = N.parent(this._id); if (!p) return null; const c = N.children(p); const i = c.indexOf(this._id); return i < 0 ? null : wrap(c[i + d] ?? 0); }
    get nextSibling() { return this._sib(1); }
    get previousSibling() { return this._sib(-1); }
    get isConnected() { for (let n = this; n; n = n.parentNode) if (n.nodeType === 9) return true; return false; }
    get textContent() { return N.text(this._id); }
    set textContent(v) {
      if (this.nodeType === 3) { this.data = v; return; }
      const gone = observing.length ? this.childNodes : null;
      N.setText(this._id, v == null ? '' : String(v)); bump();
      if (gone) record({ type: 'childList', target: this, removedNodes: gone, addedNodes: this.childNodes });
    }
    hasChildNodes() { return N.children(this._id).length > 0; }
    contains(o) { for (let n = o; n; n = n.parentNode) if (n === this) return true; return false; }
    getRootNode() { let n = this; while (n.parentNode) n = n.parentNode; return n; }
    _ins(child, ref) {
      if (child instanceof DocumentFragment) { for (const c of child._kids.splice(0)) this._ins(c, ref); return child; }
      const from = observing.length ? child.parentNode : null;
      N.insert(this._id, child._id, ref ? ref._id : 0);
      bump();
      if (from) record({ type: 'childList', target: from, removedNodes: [child] });
      if (observing.length) record({ type: 'childList', target: this, addedNodes: [child] });
      return child;
    }
    appendChild(c) { return this._ins(c, null); }
    insertBefore(c, ref) { return this._ins(c, ref); }
    removeChild(c) { c.remove(); return c; }
    replaceChild(c, old) { this._ins(c, old); old.remove(); return old; }
    remove() { const p = observing.length ? this.parentNode : null; N.remove(this._id); bump(); if (p) record({ type: 'childList', target: p, removedNodes: [this] }); }
    cloneNode(deep) { return wrap(N.clone(this._id, !!deep)); }
    isSameNode(o) { return o === this; }
    isEqualNode(o) { return o === this; }
    compareDocumentPosition(o) { return this === o ? 0 : this.contains(o) ? 20 : o.contains(this) ? 10 : 4; }
  }
  for (const [k, v] of Object.entries({ ELEMENT_NODE: 1, TEXT_NODE: 3, COMMENT_NODE: 8, DOCUMENT_NODE: 9, DOCUMENT_FRAGMENT_NODE: 11 })) { Node[k] = v; Node.prototype[k] = v; }

  class CharacterData extends Node {
    get data() { return N.data(this._id); }
    set data(v) { const old = observing.length ? this.data : null; N.setText(this._id, String(v)); if (observing.length) record({ type: 'characterData', target: this, oldValue: old }); }
    get nodeValue() { return this.data; } set nodeValue(v) { this.data = v; }
    get length() { return this.data.length; }
  }
  // `new Text('…')` and `new Comment('…')` (as libraries write them) make new nodes; wrap() passes an id.
  class Text extends CharacterData {
    constructor(x) { super(typeof x === 'number' ? x : N.createText(x == null ? '' : String(x))); nodes.set(this._id, this); }
    get nodeName() { return '#text'; } get wholeText() { return this.data; }
  }
  class Comment extends CharacterData {
    constructor(x) { super(typeof x === 'number' ? x : N.createComment(x == null ? '' : String(x))); nodes.set(this._id, this); }
    get nodeName() { return '#comment'; } get data() { return ''; } set data(v) {}
  }

  // A fragment lives only here: its children (detached nodes) move into place when it is inserted.
  class DocumentFragment extends EventTarget {
    constructor() { super(); this._kids = []; }
    get nodeType() { return 11; }
    get nodeName() { return '#document-fragment'; }
    get childNodes() { return this._kids.slice(); }
    get firstChild() { return this._kids[0] ?? null; }
    get lastChild() { return this._kids[this._kids.length - 1] ?? null; }
    get children() { return this._kids.filter((k) => k.nodeType === 1); }
    get parentNode() { return null; }
    appendChild(c) { if (c instanceof DocumentFragment) { this._kids.push(...c._kids.splice(0)); return c; } N.remove(c._id); this._kids.push(c); return c; }
    insertBefore(c, ref) { N.remove(c._id); const i = ref ? this._kids.indexOf(ref) : -1; i < 0 ? this._kids.push(c) : this._kids.splice(i, 0, c); return c; }
    removeChild(c) { const i = this._kids.indexOf(c); if (i >= 0) this._kids.splice(i, 1); return c; }
    append(...a) { for (const x of a) this.appendChild(typeof x === 'string' ? G.document.createTextNode(x) : x); }
    querySelector(s) { for (const k of this._kids) { if (k.nodeType !== 1) continue; if (k.matches(s)) return k; const f = k.querySelector(s); if (f) return f; } return null; }
    querySelectorAll(s) { const r = []; for (const k of this._kids) { if (k.nodeType !== 1) continue; if (k.matches(s)) r.push(k); r.push(...k.querySelectorAll(s)); } return r; }
    cloneNode(deep) { const f = new DocumentFragment(); if (deep) for (const k of this._kids) f._kids.push(k.cloneNode(true)); return f; }
    get textContent() { return this._kids.map((k) => k.textContent).join(''); }
  }

  class ClassList {
    constructor(el) { this._el = el; }
    _get() { return (N.attr(this._el._id, 'class') || '').split(/\s+/).filter((x) => x); }
    _set(a) { setA(this._el._id, 'class', a.join(' ')); }
    get length() { return this._get().length; }
    get value() { return N.attr(this._el._id, 'class') || ''; }
    item(i) { return this._get()[i] ?? null; }
    contains(c) { return this._get().includes(c); }
    add(...cs) { const a = this._get(); let ch = false; for (const c of cs) if (!a.includes(c)) { a.push(c); ch = true; } if (ch) this._set(a); }
    remove(...cs) { const a = this._get(); const b = a.filter((c) => !cs.includes(c)); if (b.length !== a.length) this._set(b); }
    toggle(c, force) { const has = this.contains(c); const on = force === undefined ? !has : !!force; if (on && !has) this.add(c); else if (!on && has) this.remove(c); return on; }
    replace(a, b) { const l = this._get(); const i = l.indexOf(a); if (i < 0) return false; l[i] = b; this._set(l); return true; }
    forEach(f) { this._get().forEach(f); }
    [Symbol.iterator]() { return this._get()[Symbol.iterator](); }
    toString() { return this.value; }
  }

  // element.style: setting a property sets it inline (the page restyles); reading gives what was set here.
  function styleOf(el) {
    const vals = new Map();
    const set = (name, v) => { name = kebab(name); v = v == null ? '' : String(v); if (v === '') vals.delete(name); else vals.set(name, v); N.setStyle(el._id, name, v); if (observing.length) record({ type: 'attributes', target: el, attributeName: 'style' }); };
    const api = {
      setProperty: (n, v) => set(n, v),
      removeProperty: (n) => { const old = vals.get(kebab(n)) ?? ''; set(n, ''); return old; },
      getPropertyValue: (n) => vals.get(kebab(n)) ?? '',
      item: (i) => [...vals.keys()][i] ?? '',
    };
    return new Proxy(api, {
      get(t, k) {
        if (k in t) return t[k];
        if (k === 'length') return vals.size;
        if (k === 'cssText') return [...vals].map(([n, v]) => `${n}: ${v};`).join(' ');
        if (typeof k === 'string') return vals.get(kebab(k)) ?? '';
        return undefined;
      },
      set(t, k, v) {
        if (k === 'cssText') { for (const n of [...vals.keys()]) set(n, ''); for (const d of String(v).split(';')) { const i = d.indexOf(':'); if (i > 0) set(d.slice(0, i).trim(), d.slice(i + 1).trim()); } return true; }
        set(k, v); return true;
      },
    });
  }

  const SVG_NS = 'http://www.w3.org/2000/svg';
  class Element extends Node {
    get nodeName() { return this.tagName; }
    get localName() { return N.tag(this._id); }
    get tagName() { return this.localName.toUpperCase(); }
    get namespaceURI() { return this._ns || 'http://www.w3.org/1999/xhtml'; }
    get id() { return N.attr(this._id, 'id') || ''; } set id(v) { setA(this._id, 'id', String(v)); }
    get className() { return N.attr(this._id, 'class') || ''; } set className(v) { setA(this._id, 'class', String(v)); }
    get classList() { return (this._cl ??= new ClassList(this)); }
    get style() { return (this._st ??= styleOf(this)); }
    set style(v) { this.style.cssText = v; }
    get dataset() {
      return (this._ds ??= new Proxy({}, {
        get: (_, k) => typeof k === 'string' ? N.attr(this._id, 'data-' + kebab(k)) ?? undefined : undefined,
        set: (_, k, v) => { setA(this._id, 'data-' + kebab(k), String(v)); return true; },
        deleteProperty: (_, k) => { rmA(this._id, 'data-' + kebab(k)); return true; },
        has: (_, k) => N.attr(this._id, 'data-' + kebab(k)) != null,
        ownKeys: () => this.getAttributeNames().filter((n) => n.startsWith('data-')).map((n) => camel(n.slice(5))),
        getOwnPropertyDescriptor: () => ({ enumerable: true, configurable: true }),
      }));
    }
    get hidden() { return this.hasAttribute('hidden'); } set hidden(v) { this.toggleAttribute('hidden', !!v); }
    get draggable() { return this.getAttribute('draggable') === 'true'; } set draggable(v) { this.setAttribute('draggable', v ? 'true' : 'false'); }
    get title() { return this.getAttribute('title') || ''; } set title(v) { this.setAttribute('title', v); }
    getAttribute(n) { return N.attr(this._id, n.toLowerCase()) ?? null; }
    getAttributeNS(_, n) { return this.getAttribute(n); }
    setAttribute(n, v) { setA(this._id, n.toLowerCase(), String(v)); }
    setAttributeNS(_, n, v) { this.setAttribute(n.replace(/^xlink:/, ''), v); }
    removeAttribute(n) { rmA(this._id, n.toLowerCase()); }
    removeAttributeNS(_, n) { this.removeAttribute(n); }
    hasAttribute(n) { return N.attr(this._id, n.toLowerCase()) != null; }
    hasAttributes() { return N.attrs(this._id).length > 0; }
    toggleAttribute(n, force) { const on = force === undefined ? !this.hasAttribute(n) : !!force; on ? this.setAttribute(n, '') : this.removeAttribute(n); return on; }
    getAttributeNames() { const a = N.attrs(this._id); const r = []; for (let i = 0; i < a.length; i += 2) r.push(a[i]); return r; }
    get attributes() { const a = N.attrs(this._id); const r = []; for (let i = 0; i < a.length; i += 2) r.push({ name: a[i], localName: a[i], value: a[i + 1], nodeName: a[i], nodeValue: a[i + 1] }); r.item = (i) => r[i] ?? null; r.getNamedItem = (n) => r.find((x) => x.name === n) ?? null; return r; }
    get children() { return this.childNodes.filter((c) => c.nodeType === 1); }
    get childElementCount() { return this.children.length; }
    get firstElementChild() { return this.children[0] ?? null; }
    get lastElementChild() { const c = this.children; return c[c.length - 1] ?? null; }
    get nextElementSibling() { let n = this.nextSibling; while (n && n.nodeType !== 1) n = n.nextSibling; return n; }
    get previousElementSibling() { let n = this.previousSibling; while (n && n.nodeType !== 1) n = n.previousSibling; return n; }
    querySelector(s) { return wrap(N.q(this._id, s, true)[0] ?? 0); }
    querySelectorAll(s) { return list(N.q(this._id, s, false)); }
    getElementsByTagName(t) { return this.querySelectorAll(t); }
    getElementsByClassName(c) { return this.querySelectorAll(c.split(/\s+/).filter((x) => x).map((x) => '.' + x).join('')); }
    matches(s) { return N.matches(this._id, s); }
    closest(s) { for (let n = this; n && n.nodeType === 1; n = n.parentNode) if (n.matches(s)) return n; return null; }
    get innerHTML() { return this.childNodes.map(html).join(''); }
    set innerHTML(v) {
      const gone = observing.length ? this.childNodes : null;
      N.setHtml(this._id, String(v)); bump();
      if (gone) record({ type: 'childList', target: this, removedNodes: gone, addedNodes: this.childNodes });
    }
    get outerHTML() { return html(this); }
    get innerText() { return this.textContent; } set innerText(v) { this.textContent = v; }
    getBoundingClientRect() { const [x, y, w, h] = N.rect(this._id); return { x, y, width: w, height: h, left: x, top: y, right: x + w, bottom: y + h }; }
    getClientRects() { return [this.getBoundingClientRect()]; }
    get offsetWidth() { return N.rect(this._id)[2]; } get offsetHeight() { return N.rect(this._id)[3]; }
    get clientWidth() { return this.offsetWidth; } get clientHeight() { return this.offsetHeight; }
    get offsetLeft() { return N.rect(this._id)[0]; } get offsetTop() { return N.rect(this._id)[1]; }
    append(...a) { for (const x of a) this.appendChild(typeof x === 'string' ? G.document.createTextNode(x) : x); }
    prepend(...a) { const f = this.firstChild; for (const x of a) this.insertBefore(typeof x === 'string' ? G.document.createTextNode(x) : x, f); }
    before(...a) { const p = this.parentNode; if (p) for (const x of a) p.insertBefore(typeof x === 'string' ? G.document.createTextNode(x) : x, this); }
    after(...a) { const p = this.parentNode; if (!p) return; const nx = this.nextSibling; for (const x of a) p.insertBefore(typeof x === 'string' ? G.document.createTextNode(x) : x, nx); }
    replaceWith(...a) { this.before(...a); this.remove(); }
    replaceChildren(...a) { for (const c of this.childNodes) c.remove(); this.append(...a); }
    insertAdjacentElement(where, el) { ({ beforebegin: () => this.before(el), afterbegin: () => this.prepend(el), beforeend: () => this.append(el), afterend: () => this.after(el) })[where.toLowerCase()]?.(); return el; }
    insertAdjacentText(where, t) { this.insertAdjacentElement(where, G.document.createTextNode(t)); }
    focus() { const was = G.document.activeElement; N.focus(this._id); const now = G.document.activeElement; if (now === was) return;
      for (const t of ['blur', 'focusout']) was?.dispatchEvent(new FocusEvent(t, { bubbles: t === 'focusout' }));
      for (const t of ['focus', 'focusin']) now?.dispatchEvent(new FocusEvent(t, { bubbles: t === 'focusin' })); }
    blur() { if (G.document.activeElement !== this) return; N.blur(); for (const t of ['blur', 'focusout']) this.dispatchEvent(new FocusEvent(t, { bubbles: t === 'focusout' })); }
    click() { this.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true })); }
    scrollIntoView() {}
    animate(frames, options) { return new Animation(this, frames, options); }
    getAnimations() { return (this._wa || []).slice(); }
    get content() { if (this.localName !== 'template') return undefined; const f = new DocumentFragment(); for (const c of this.childNodes) f._kids.push(c.cloneNode(true)); return f; }
    get value() { return N.value(this._id) ?? ''; } set value(v) { if (this.localName === 'select') { const o = this.options.find((x) => x.value === String(v)); if (o) o.selected = true; } else this.setAttribute('value', v); }
    get checked() { return this._checked ?? this.hasAttribute('checked'); } set checked(v) { this._checked = !!v; this.toggleAttribute('checked', !!v); }
    get disabled() { return this.hasAttribute('disabled'); } set disabled(v) { this.toggleAttribute('disabled', !!v); }
    // <select> and <option>: the chosen option is the library's (the user's choice), not the `selected` attribute.
    get options() { return this.localName === 'select' ? this.querySelectorAll('option') : undefined; }
    get selectedOptions() { return this.localName === 'select' ? this.options.filter((o) => o.selected) : undefined; }
    get selectedIndex() { return this.localName === 'select' ? this.options.findIndex((o) => o.selected) : undefined; }
    set selectedIndex(i) { const o = this.options?.[i]; if (o) o.selected = true; }
    get selected() { return N.checked(this._id); }
    set selected(v) { if (!v) return; const s = this.closest('select'); if (s) N.pick(s._id, this._id); }
    get text() { return this.localName === 'option' ? this.textContent.trim().replace(/\s+/g, ' ') : this.textContent; }
    get index() { const s = this.closest('select'); return s ? s.options.indexOf(this) : 0; }
    get length() { return this.localName === 'select' ? this.options.length : undefined; }
  }
  class HTMLElement extends Element {}
  // Event handler properties (`el.onclick = …`): libraries test `'onclick' in el` to find an event's name.
  for (const t of ['click', 'dblclick', 'contextmenu', 'mousedown', 'mouseup', 'mousemove', 'mouseover', 'mouseout', 'mouseenter', 'mouseleave',
    'pointerdown', 'pointerup', 'pointermove', 'pointerover', 'pointerout', 'pointerenter', 'pointerleave', 'pointercancel', 'wheel',
    'keydown', 'keyup', 'keypress', 'input', 'change', 'submit', 'focus', 'blur', 'focusin', 'focusout', 'scroll', 'load', 'error',
    'transitionend', 'animationend', 'animationstart', 'animationiteration']) {
    const key = 'on' + t;
    Object.defineProperty(EventTarget.prototype, key, {
      get() { return this._on?.[t] ?? null; },
      set(fn) { (this._on ??= {})[t] = fn; if (fn) listen(t); },
      configurable: true,
    });
  }

  class Document extends Node {
    get nodeName() { return '#document'; }
    get documentElement() { return this.childNodes.find((c) => c.nodeType === 1) ?? null; }
    get head() { return this.querySelector('head'); }
    get body() { return this.querySelector('body'); }
    get defaultView() { return G; }
    get activeElement() { return wrap(N.focused()) || this.body; }
    get readyState() { return G.__ready ? 'complete' : 'loading'; }
    get location() { return G.location; }
    createElement(t) { return wrap(N.create(String(t), false)); }
    createElementNS(ns, t) { const e = wrap(N.create(String(t), ns === SVG_NS)); if (ns === SVG_NS) e._ns = SVG_NS; return e; }
    createTextNode(t) { return wrap(N.createText(String(t))); }
    createComment(t) { return wrap(N.createComment(String(t))); }
    createDocumentFragment() { return new DocumentFragment(); }
    createEvent() { return new Event(''); }
    getElementById(i) { return this.querySelector('#' + CSS.escape(i)); }
    querySelector(s) { return wrap(N.q(this._id, s, true)[0] ?? 0); }
    querySelectorAll(s) { return list(N.q(this._id, s, false)); }
    getElementsByTagName(t) { return this.querySelectorAll(t); }
    getElementsByClassName(c) { return this.querySelectorAll(c.split(/\s+/).filter((x) => x).map((x) => '.' + x).join('')); }
    hasFocus() { return true; }
  }

  // ---------- Serializing (innerHTML, outerHTML) ----------
  const VOID = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'source', 'track', 'wbr']);
  const esc = (t, attr) => String(t).replace(/&/g, '&amp;').replace(attr ? /"/g : /[<>]/g, (c) => ({ '"': '&quot;', '<': '&lt;', '>': '&gt;' })[c]);
  function html(n) {
    if (n.nodeType === 3) { const p = n.parentNode?.localName; return p === 'style' || p === 'script' ? n.data : esc(n.data); }
    if (n.nodeType === 8) return `<!--${n.data}-->`;
    if (n.nodeType !== 1) return '';
    const tag = n.localName;
    const a = N.attrs(n._id);
    let s = '<' + tag;
    for (let i = 0; i < a.length; i += 2) s += a[i + 1] === '' ? ` ${a[i]}` : ` ${a[i]}="${esc(a[i + 1], true)}"`;
    s += '>';
    if (VOID.has(tag)) return s;
    return s + n.childNodes.map(html).join('') + `</${tag}>`;
  }

  // ---------- fetch: files next to the page ----------
  class Headers { constructor(h) { this._h = new Map(Object.entries(h || {}).map(([k, v]) => [k.toLowerCase(), String(v)])); } get(k) { return this._h.get(k.toLowerCase()) ?? null; } has(k) { return this._h.has(k.toLowerCase()); } forEach(f) { this._h.forEach((v, k) => f(v, k, this)); } }
  class Response {
    constructor(body, init = {}) { this._body = body == null ? '' : String(body); this.status = init.status ?? 200; this.statusText = init.statusText ?? ''; this.ok = this.status >= 200 && this.status < 300; this.url = init.url ?? ''; this.headers = new Headers(init.headers); this.bodyUsed = false; this.type = 'basic'; this.redirected = false; }
    text() { this.bodyUsed = true; return Promise.resolve(this._body); }
    json() { return this.text().then(JSON.parse); }
    clone() { return new Response(this._body, this); }
  }
  // Only files next to the page (or file: URLs) can be read; a missing file is a 404, other URLs fail as offline.
  function fetch(input, init) {
    const url = String(typeof input === 'object' && input ? input.url : input);
    return new Promise((ok, no) => Promise.resolve().then(() => {
      if (/^[a-z]+:/i.test(url) && !url.startsWith('file:')) { no(new TypeError(`fetch: only files next to the page can be read (${url})`)); return; }
      const text = N.read(url);
      const type = url.endsWith('.json') ? 'application/json' : url.endsWith('.html') ? 'text/html' : url.endsWith('.css') ? 'text/css' : 'text/plain';
      ok(text == null ? new Response('', { status: 404, statusText: 'Not Found', url }) : new Response(text, { status: 200, statusText: 'OK', url, headers: { 'content-type': type } }));
    }));
  }

  // ---------- Web Animations (el.animate) over CSS animations ----------
  // The keyframes become an @keyframes rule in one style element of the page, and the element's inline `animation`
  // lists its running web animations; the library's animation engine (and its compositor, for transform and opacity)
  // plays them, and their animationend finishes them.
  let animSeq = 0;
  let animStyle = null;
  function keyframesRule(name, frames) {
    let list;
    if (Array.isArray(frames)) list = frames.map((f) => ({ ...f }));
    else {
      const keys = Object.keys(frames || {}).filter((k) => k !== 'offset' && k !== 'easing' && k !== 'composite');
      const n = Math.max(1, ...keys.map((k) => [].concat(frames[k]).length));
      list = [];
      for (let i = 0; i < n; i++) { const f = {}; for (const k of keys) { const v = [].concat(frames[k]); f[k] = v[Math.min(i, v.length - 1)]; } list.push(f); }
    }
    const n = list.length;
    list.forEach((f, i) => { if (f.offset == null) f.offset = n === 1 ? 1 : i / (n - 1); });
    const body = list.map((f) => {
      const decl = Object.entries(f).filter(([k]) => k !== 'offset' && k !== 'easing' && k !== 'composite').map(([k, v]) => `${kebab(k)}: ${v};`).join(' ');
      return `${+(f.offset * 100).toFixed(4)}% { ${decl}${f.easing ? ` animation-timing-function: ${f.easing};` : ''} }`;
    }).join(' ');
    return `@keyframes ${name} { ${body} }`;
  }
  class Animation extends EventTarget {
    constructor(el, frames, options) {
      super();
      const o = typeof options === 'number' ? { duration: options } : (options || {});
      this.effect = { target: el, getTiming: () => o };
      this.id = o.id || '';
      this._el = el;
      this._name = '__wa' + (++animSeq);
      const fill = o.fill && o.fill !== 'auto' ? o.fill : 'none';
      const iterations = o.iterations === Infinity ? 'infinite' : (o.iterations ?? 1);
      this._css = `${this._name} ${(o.duration ?? 0) / 1000}s ${o.easing || 'linear'} ${(o.delay ?? 0) / 1000}s ${iterations} ${o.direction || 'normal'} ${fill}`;
      this._keep = fill === 'forwards' || fill === 'both';
      this.playState = 'running';
      this.onfinish = null; this.oncancel = null;
      this._start = N.now();
      this.finished = new Promise((ok, no) => { this._ok = ok; this._no = no; });
      this.finished.catch(() => {});
      if (!animStyle) { animStyle = G.document.createElement('style'); (G.document.head || G.document.documentElement).appendChild(animStyle); }
      animStyle.appendChild(G.document.createTextNode(keyframesRule(this._name, frames) + '\n'));
      this._onEnd = (e) => { if (e.animationName === this._name && e.target === el) this._finish(); };
      el.addEventListener('animationend', this._onEnd);
      (el._wa ??= []).push(this);
      this._apply();
    }
    _apply() {
      const el = this._el;
      el.style.animation = (el._wa || []).map((a) => a._css).join(', ');
      el.style.animationPlayState = (el._wa || []).map((a) => a.playState === 'paused' ? 'paused' : 'running').join(', ');
    }
    _drop() {
      const el = this._el;
      el.removeEventListener('animationend', this._onEnd);
      if (el._wa) el._wa = el._wa.filter((a) => a !== this);
      this._apply();
    }
    _finish() {
      if (this.playState === 'finished') return;
      this.playState = 'finished';
      if (!this._keep) this._drop(); else this._el.removeEventListener('animationend', this._onEnd);
      this.dispatchEvent(new Event('finish')); // calls onfinish too
      this._ok(this);
    }
    get currentTime() { return this.playState === 'idle' ? null : N.now() - this._start; }
    get pending() { return false; }
    play() { if (this.playState === 'paused') { this.playState = 'running'; this._apply(); } }
    pause() { if (this.playState === 'running') { this.playState = 'paused'; this._apply(); } }
    cancel() {
      if (this.playState === 'idle') return;
      this.playState = 'idle';
      this._drop();
      this.dispatchEvent(new Event('cancel'));
      const err = new Error('The animation was canceled'); err.name = 'AbortError';
      this._no(err);
    }
    finish() { this._finish(); }
    commitStyles() {}
    persist() {}
  }

  // ---------- Timers (the page clock is the game's, set by vitrail_tick) ----------
  let timerSeq = 1;
  const timers = new Map();
  let frames = [];
  let frameSeq = 1;
  function wake() {
    let at = frames.length ? 0 : Infinity;
    for (const t of timers.values()) if (t.at < at) at = t.at;
    N.wake(at);
  }
  function addTimer(fn, ms, args, repeat) {
    const id = timerSeq++;
    ms = Math.max(0, +ms || 0);
    timers.set(id, { fn, args, at: N.now() + ms, every: repeat ? Math.max(ms, 1) : 0 });
    wake();
    return id;
  }
  G.setTimeout = (fn, ms, ...args) => addTimer(fn, ms, args, false);
  G.setInterval = (fn, ms, ...args) => addTimer(fn, ms, args, true);
  G.clearTimeout = G.clearInterval = (id) => { timers.delete(id); wake(); };
  G.requestAnimationFrame = (fn) => { const id = frameSeq++; frames.push({ id, fn }); N.wake(0); return id; };
  G.cancelAnimationFrame = (id) => { frames = frames.filter((f) => f.id !== id); wake(); };
  G.requestIdleCallback = (fn) => G.setTimeout(() => fn({ didTimeout: false, timeRemaining: () => 10 }), 1);
  G.cancelIdleCallback = G.clearTimeout;
  G.queueMicrotask = (fn) => { Promise.resolve().then(fn); };
  G.__tick = (now) => {
    for (let guard = 0; guard < 1000; guard++) {
      let next = null;
      for (const [id, t] of timers) if (t.at <= now && (!next || t.at < next[1].at)) next = [id, t];
      if (!next) break;
      const [id, t] = next;
      if (t.every) t.at += t.every * Math.max(1, Math.ceil((now - t.at + 1e-9) / t.every)); else timers.delete(id);
      try { typeof t.fn === 'function' ? t.fn(...t.args) : (0, eval)(String(t.fn)); } catch (err) { reportError(err); }
    }
    const run = frames; frames = [];
    for (const f of run) { try { f.fn(now); } catch (err) { reportError(err); } }
    wake();
  };

  // ---------- Console ----------
  const fmt = (a) => a.map((x) => typeof x === 'string' ? x : x instanceof Error ? `${x.name}: ${x.message}` : (() => { try { return JSON.stringify(x); } catch { return String(x); } })()).join(' ');
  function reportError(err) { N.log(2, err instanceof Error ? `${err.name}: ${err.message}\n${err.stack || ''}` : 'Uncaught ' + fmt([err])); }
  G.reportError = reportError;
  G.console = {
    log: (...a) => N.log(0, fmt(a)), info: (...a) => N.log(0, fmt(a)), debug: (...a) => N.log(0, fmt(a)),
    warn: (...a) => N.log(1, fmt(a)), error: (...a) => N.log(2, fmt(a)), trace: (...a) => N.log(0, fmt(a)),
    assert: (c, ...a) => { if (!c) N.log(2, 'Assertion failed: ' + fmt(a)); }, group() {}, groupEnd() {}, time() {}, timeEnd() {}, table: (x) => N.log(0, fmt([x])),
  };

  // ---------- The game's bridge (as Coherent's `engine`) ----------
  const handlers = new Map();
  G.engine = {
    on(name, fn, self) { let a = handlers.get(name); if (!a) handlers.set(name, (a = [])); a.push({ fn, self }); },
    off(name, fn) { const a = handlers.get(name); if (a) handlers.set(name, a.filter((h) => h.fn !== fn)); },
    trigger(name, ...args) { N.emit(String(name), JSON.stringify(args)); },
    whenReady: Promise.resolve(),
  };
  G.__engineCall = (name, json) => {
    const a = handlers.get(name);
    if (!a) return;
    let args = [];
    try { args = JSON.parse(json); if (!Array.isArray(args)) args = [args]; } catch (err) { reportError(err); return; }
    for (const h of a.slice()) { try { h.fn.apply(h.self, args); } catch (err) { reportError(err); } }
  };

  // ---------- Input from the game ----------
  // HTML drag and drop over the mouse's events, as browsers do it: a press on a [draggable=true] element that moves
  // 4 px starts it (dragstart, cancelable); then drag on the source, dragenter/dragover/dragleave on what is under the
  // pointer (a dragover whose default is prevented allows the drop there), drop on release over such an element,
  // dragend on the source, and the click that the release would make is swallowed. A copy of the source (or the
  // setDragImage element) follows the pointer at 70% opacity, pointer-events: none.
  const dnd = { src: null, x: 0, y: 0, on: false, over: null, ok: false, dt: null, ghost: null, gx: 0, gy: 0, eat: false };
  const dragEvent = (type, target, x, y) => {
    const e = new DragEvent(type, { bubbles: true, cancelable: type !== 'dragleave' && type !== 'dragend', clientX: x, clientY: y, dataTransfer: dnd.dt });
    e.isTrusted = true;
    return target.dispatchEvent(e);
  };
  function dragEnd() { dnd.ghost?.remove(); dnd.src = null; dnd.on = false; dnd.over = null; dnd.ok = false; dnd.dt = null; dnd.ghost = null; }
  function drag(type, t, x, y) {
    if (type === 'click' && dnd.eat) { dnd.eat = false; return true; }
    if (type === 'mousedown') { dragEnd(); dnd.src = t.closest?.('[draggable="true"]') ?? null; dnd.x = x; dnd.y = y; return false; }
    if (!dnd.src) return false;
    if (type === 'mousemove') {
      if (!dnd.on) {
        if (Math.hypot(x - dnd.x, y - dnd.y) < 4) return false;
        dnd.dt = new DataTransfer();
        if (!dragEvent('dragstart', dnd.src, dnd.x, dnd.y)) { dragEnd(); return false; }
        dnd.on = true;
        const img = dnd.dt._image, from = img?.el ?? dnd.src, r = dnd.src.getBoundingClientRect();
        dnd.gx = img ? img.x : dnd.x - r.left; dnd.gy = img ? img.y : dnd.y - r.top;
        const g = from.cloneNode(true);
        g.removeAttribute('id');
        const fr = from.getBoundingClientRect();
        Object.assign(g.style, { position: 'fixed', margin: '0', width: fr.width + 'px', height: fr.height + 'px', opacity: '0.7', pointerEvents: 'none', zIndex: '2147483647' });
        G.document.body.appendChild(g);
        dnd.ghost = g;
      }
      dnd.ghost.style.left = (x - dnd.gx) + 'px'; dnd.ghost.style.top = (y - dnd.gy) + 'px';
      dragEvent('drag', dnd.src, x, y);
      if (t !== dnd.over) { if (dnd.over) dragEvent('dragleave', dnd.over, x, y); dragEvent('dragenter', t, x, y); dnd.over = t; }
      dnd.dt.dropEffect = 'none';
      dnd.ok = !dragEvent('dragover', t, x, y);
      if (dnd.ok && dnd.dt.dropEffect === 'none') dnd.dt.dropEffect = 'move';
      return true;
    }
    if (type === 'mouseup') {
      if (!dnd.on) { dnd.src = null; return false; }
      if (dnd.ok && dnd.over) dragEvent('drop', dnd.over, x, y); else dnd.dt.dropEffect = 'none';
      const src = dnd.src;
      dragEvent('dragend', src, x, y);
      dragEnd();
      dnd.eat = true;
      return false;
    }
    return false;
  }

  G.__dispatch = (type, target, x, y, button, extra) => {
    const t = wrap(target);
    if (!t) return true;
    // While dragging, the mouse's moves are the drag's (not mousemove); a press or release still reaches the page.
    if ((type === 'mousedown' || type === 'mousemove' || type === 'mouseup' || type === 'click') && drag(type, t, x, y)) return true;
    const init = { bubbles: !['mouseenter', 'mouseleave', 'pointerenter', 'pointerleave', 'focus', 'blur'].includes(type), cancelable: true, clientX: x, clientY: y, button, buttons: button >= 0 ? 1 : 0, ...(extra ? JSON.parse(extra) : {}) };
    const E = type.startsWith('pointer') ? PointerEvent : type === 'wheel' ? WheelEvent : type.startsWith('key') ? KeyboardEvent
      : type.startsWith('transition') ? TransitionEvent : type.startsWith('animation') ? AnimationEvent : MouseEvent;
    const e = new E(type, init);
    e.isTrusted = true;
    return t.dispatchEvent(e);
  };

  // Element kinds by tag: one Element class serves all, `x instanceof HTMLTemplateElement` asks the tag.
  const SVG_TAGS = new Set(['svg', 'g', 'path', 'circle', 'rect', 'line', 'polyline', 'polygon', 'ellipse', 'text', 'tspan', 'defs', 'use', 'linearGradient', 'radialGradient', 'stop', 'clipPath', 'mask', 'pattern', 'image', 'symbol']);
  function kind(test) {
    const K = function () { throw new TypeError('Illegal constructor'); };
    K.prototype = Element.prototype;
    Object.defineProperty(K, Symbol.hasInstance, { value: (x) => x instanceof Element && test(x) });
    return K;
  }

  // ---------- The window ----------
  // dom.js runs before the page is reachable: the document and the viewport are read on first use.
  Object.setPrototypeOf(G, EventTarget.prototype);
  let doc = null;
  Object.defineProperty(G, 'document', { get: () => (doc ??= wrap(N.doc())), configurable: true });
  Object.defineProperty(G, 'devicePixelRatio', { get: () => N.viewport()[2], configurable: true });
  Object.assign(G, {
    window: G, self: G, globalThis: G,
    Animation, Node, Element, HTMLElement, Text, Comment, CharacterData, Document, DocumentFragment, EventTarget,
    Event, CustomEvent, UIEvent, MouseEvent, PointerEvent, WheelEvent, KeyboardEvent, FocusEvent, InputEvent, TransitionEvent, AnimationEvent, DragEvent, DataTransfer,
    SVGElement: kind((e) => e._ns === SVG_NS || SVG_TAGS.has(e.localName)),
    HTMLTemplateElement: kind((e) => e.localName === 'template'), HTMLInputElement: kind((e) => e.localName === 'input'),
    HTMLTextAreaElement: kind((e) => e.localName === 'textarea'), HTMLSelectElement: kind((e) => e.localName === 'select'),
    HTMLButtonElement: kind((e) => e.localName === 'button'), HTMLDivElement: kind((e) => e.localName === 'div'),
    HTMLImageElement: kind((e) => e.localName === 'img'), HTMLAnchorElement: kind((e) => e.localName === 'a'),
    navigator: { userAgent: 'vitrail (Blitz; QuickJS)', language: 'en', languages: ['en'], platform: '' },
    location: { href: '', protocol: 'file:', host: '', hostname: '', pathname: '', search: '', hash: '', origin: 'null', reload() {}, assign() {}, replace() {} },
    history: { pushState() {}, replaceState() {}, back() {}, forward() {}, length: 1, state: null },
    performance: { now: () => N.now(), mark() {}, measure() {}, timeOrigin: 0 },
    // Computed values from the style engine, read as properties (camelCase or dashed) or with getPropertyValue.
    getComputedStyle: (el) => new Proxy({}, { get: (_, k) => {
      if (k === 'getPropertyValue') return (p) => N.computed(el._id, String(p));
      if (typeof k !== 'string') return undefined;
      return N.computed(el._id, k.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase()));
    } }),
    matchMedia: (q) => ({ matches: false, media: q, onchange: null, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} }),
    CSS: { escape: (s) => String(s).replace(/([^\w-])/g, '\\$1'), supports: () => true },
    AbortController: class { constructor() { this.signal = new (class extends EventTarget { aborted = false; })(); } abort() { this.signal.aborted = true; this.signal.dispatchEvent(new Event('abort')); } },
    MutationObserver, fetch, Response, Headers,
    ResizeObserver: class { observe() {} unobserve() {} disconnect() {} },
    IntersectionObserver: class { observe() {} unobserve() {} disconnect() {} },
  });
  Object.defineProperty(G, 'innerWidth', { get: () => N.viewport()[0] });
  Object.defineProperty(G, 'innerHeight', { get: () => N.viewport()[1] });
})();
