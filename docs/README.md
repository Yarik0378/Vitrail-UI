# vitrail — HTML and CSS game interfaces, drawn by the game's GPU

vitrail lets a game's interface be written in HTML and CSS and drawn inside the game by its own GPU: no browser, no
WebView, no Chromium. The CSS engine is Firefox's (Stylo, through [Blitz](https://github.com/DioxusLabs/blitz)), the
layout is Taffy's (flex, grid, block), the text is Parley's. vitrail adds the bridge to the engine, a GPU painter that
draws the page as triangles, and everything a game needs around it: rendering only what changed, work off the main
thread, glass (backdrop-filter), filters, blend modes, CSS animations with a compositor, game textures inside pages.

It is an alternative to game UI middleware such as Coherent Gameface. Today it runs in Unity 6
(URP) on macOS and Windows; Godot and Unreal are planned on the same Rust core.

**Status:** early (0.1). Rendering, animations, JavaScript, input, the Unity package, developer tools and CI work;
see the [roadmap](#roadmap).

## Documents

| Document | For |
|---|---|
| [Getting started in Unity](getting-started.md) | Showing a page in a game, talking to it from C# |
| [CSS support](css-support.md) | What works, how close to Chrome, what doesn't |
| [Architecture](architecture.md) | How a page becomes pixels: threads, the GPU painter, layers, glass, the compositor |
| [C API](c-api.md) | Every `vitrail_*` function of the native library (for engine integrations) |
| [Performance](performance.md) | Measured costs and budgets, and how to measure |
| [Maintaining](maintaining.md) | Building the library, the patches on Blitz and anyrender, the test suite |

## How close to Chrome

Each test page is rendered by headless Chrome and by vitrail, and compared pixel by pixel (mean difference over black
and over white backgrounds, out of 255). Most pages differ by less than 1/255; see [CSS support](css-support.md).

## Roadmap

1. GPU rendering — done: edge antialiasing without MSAA, caches, filters, blend modes, masks, clip-path, glass, text
   (scaled, rotated, emoji), game textures. Left: Windows colour emoji (COLR), layer textures sized to their element.
2. Animations — done: CSS transitions and keyframes; a compositor moves transform/opacity animations without painting
   (20 at once 0.1 ms a frame); animation events and `el.animate()` for scripts. Repainting animations (colour, size)
   cost 0.2–0.4 ms for 20: cutting them needs Blitz to reuse unchanged elements' styles and draw commands.
3. JavaScript — done: QuickJS runs the page's scripts and ES modules with a DOM, events, timers, promises, console and
   the game's bridge (`engine.on` / `engine.trigger`); Preact and petite-vue run unchanged (a click and its update
   0.04–0.1 ms); Coherent-style data binding (`data-bind-*`); the keyboard, focus and typing into text fields;
   `fetch()` of local files; MutationObserver; `innerHTML`/`outerHTML`.
4. Input — done: text fields with the mouse, clipboard and IME; focus, Tab and `:focus-visible`; `<select>` with a list
   styled in CSS; gamepad (spatial) navigation; smooth wheel, scrolling on the GPU without stalls, overlay scrollbars;
   HTML drag and drop.
5. Unity package — done: `package/` (`com.vitrail.ui`): VitrailScreen and VitrailWorld (a page on a surface in the
   world), inspectors with a page picker, hot reload, four samples (menu, HUD, inventory, world panel). URP; Built-in
   and HDRP later.
6. Developer tools — done: Window > Vitrail UI > Inspector: element tree, box, attributes, computed styles (editable live),
   the element highlighted over the game, picking with the mouse; `getComputedStyle` in scripts. Chrome DevTools
   protocol later.
7. CI — done: `.github/workflows/ci.yml` builds macOS, Windows and Linux, runs the tests (retained-painting equality,
   budgets, the comparison with Chrome's references on macOS) and assembles the package with all three libraries.
8. Public release on GitHub under MIT with the Commons Clause.
