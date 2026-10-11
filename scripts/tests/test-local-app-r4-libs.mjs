import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import test from 'node:test';

const root = fileURLToPath(new URL('../../plugins/local-app-builder/assets/templates/', import.meta.url));
const revision = 'r4';
const families = ['react-dom', 'canvas-2d', 'three-3d', 'phaser-2d', 'babylon-3d'];

function load(family, file, globals = {}) {
  let source = readFileSync(`${root}${family}/${revision}/lib/${file}`, 'utf8');
  const names = [...source.matchAll(/export (?:async )?(?:function|const) (\w+)/g)].map((match) => match[1]);
  source = source.replace(/^import [\s\S]*? from [^;]+;\n/gm, '').replace(/export /g, '');
  source = source.replace(/return \(\s*<PlatformContext.Provider[\s\S]*?<\/PlatformContext.Provider>\s*\);/, 'return value;');
  const context = vm.createContext({ console, ...globals });
  return vm.runInContext(`${source}\n;({${names.join(',')}})`, context);
}

function target() {
  const listeners = new Map();
  return {
    addEventListener(name, fn) { if (!listeners.has(name)) listeners.set(name, new Set()); listeners.get(name).add(fn); },
    removeEventListener(name, fn) { listeners.get(name)?.delete(fn); },
    emit(name) { for (const fn of [...(listeners.get(name) ?? [])]) fn(); },
    count() { return [...listeners.values()].reduce((sum, set) => sum + set.size, 0); },
  };
}

function browser() {
  const frames = new Map();
  let nextFrame = 0;
  const window = Object.assign(target(), {
    devicePixelRatio: 2,
    requestAnimationFrame(fn) { frames.set(++nextFrame, fn); return nextFrame; },
    cancelAnimationFrame(id) { frames.delete(id); },
    visualViewport: target(),
  });
  const document = Object.assign(target(), { visibilityState: 'visible' });
  let observer;
  class ResizeObserver {
    constructor(callback) { this.callback = callback; observer = this; }
    observe() {}
    disconnect() { this.disconnected = true; }
  }
  const canvas = { width: 0, height: 0, getBoundingClientRect: () => ({ width: 100, height: 80 }) };
  return { window, document, ResizeObserver, canvas, frames,
    get observer() { return observer; },
    tick(timestamp) { const pending = [...frames.values()]; frames.clear(); for (const fn of pending) fn(timestamp); },
  };
}

/// Media queries answer from `matching` and are event targets, so a test can
/// flip one and fire its `change` event the way a browser does.
function mediaQueries(matching = []) {
  const lists = new Map();
  return {
    lists,
    matchMedia(query) {
      if (!lists.has(query)) lists.set(query, Object.assign(target(), { matches: matching.includes(query) }));
      return lists.get(query);
    },
  };
}

/// A desktop-class browser with no safe-area probe: `document.body` is absent,
/// so `readSafeArea` takes its fallback unless a test supplies one.
function device(family, { userAgent = '', maxTouchPoints = 0, screen = { width: 1440, height: 900 },
  matching = [], innerWidth = 1280, innerHeight = 800, visualViewport = null, document = {},
  getComputedStyle } = {}) {
  const media = mediaQueries(matching);
  const window = { innerWidth, innerHeight, screen, visualViewport, matchMedia: media.matchMedia };
  const globals = { window, navigator: { userAgent, maxTouchPoints }, document };
  if (getComputedStyle) globals.getComputedStyle = getComputedStyle;
  return load(family, 'device-context.js', globals);
}

const IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15';
const MAC = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15';
const ANDROID = 'Mozilla/5.0 (Linux; Android 16; Pixel 10) AppleWebKit/537.36 Chrome/141.0 Mobile Safari/537.36';

for (const family of families) {
  test(`${family}: device context classifies OS and form factor from the browser`, () => {
    const facts = (options) => {
      const context = device(family, options).getDeviceContext();
      return `${context.os}:${context.formFactor}`;
    };
    assert.equal(facts({ userAgent: IPHONE, maxTouchPoints: 5 }), 'ios:iphone');
    // iPadOS sends a Mac user agent; the touch points give it away.
    assert.equal(facts({ userAgent: MAC, maxTouchPoints: 5 }), 'ios:ipad');
    assert.equal(facts({ userAgent: MAC, maxTouchPoints: 0 }), 'desktop:desktop');
    assert.equal(facts({ userAgent: ANDROID, screen: { width: 412, height: 915 } }), 'android:phone');
    assert.equal(facts({ userAgent: ANDROID, screen: { width: 800, height: 1280 } }), 'android:tablet');
  });

  test(`${family}: device context reads appearance, motion, input mode and viewport`, () => {
    const dark = device(family, {
      userAgent: MAC,
      matching: ['(prefers-color-scheme: dark)', '(prefers-reduced-motion: reduce)', '(pointer: fine)', '(any-pointer: coarse)'],
      visualViewport: { width: 640, height: 480 },
    }).getDeviceContext();
    assert.equal(dark.colorScheme, 'dark');
    assert.equal(dark.reducedMotion, true);
    assert.equal(dark.inputMode, 'hybrid');
    // The visual viewport (keyboard, pinch-zoom) wins over the layout viewport.
    assert.deepEqual({ ...dark.viewport }, { width: 640, height: 480 });

    const plain = device(family, { userAgent: MAC, matching: ['(pointer: fine)'] }).getDeviceContext();
    assert.equal(plain.colorScheme, 'light');
    assert.equal(plain.reducedMotion, false);
    assert.equal(plain.inputMode, 'pointer');
    assert.deepEqual({ ...plain.viewport }, { width: 1280, height: 800 });
    assert.equal(device(family, { userAgent: IPHONE }).getDeviceContext().inputMode, 'touch');
  });

  test(`${family}: safe-area insets come from one reused probe element`, () => {
    const appended = [];
    const probe = { style: {}, setAttribute() {}, isConnected: false };
    const document = {
      body: { appendChild(element) { appended.push(element); element.isConnected = true; } },
      createElement: () => probe,
    };
    const getComputedStyle = () => ({ paddingTop: '47px', paddingRight: '0px', paddingBottom: '34px', paddingLeft: 'bogus' });
    const module = device(family, { userAgent: IPHONE, document, getComputedStyle });
    assert.deepEqual({ ...module.getDeviceContext().safeArea }, { top: 47, right: 0, bottom: 34, left: 0 });
    module.getDeviceContext();
    assert.equal(appended.length, 1);
  });

  test(`${family}: normalizeDeviceContext rejects invalid values`, () => {
    const { normalizeDeviceContext, FALLBACK_DEVICE_CONTEXT } = device(family);
    assert.equal(normalizeDeviceContext(null), FALLBACK_DEVICE_CONTEXT);
    const context = normalizeDeviceContext({
      os: 7, viewport: { width: -1, height: Number.NaN }, safeArea: { top: Infinity },
      colorScheme: 'sepia', reducedMotion: 'yes', inputMode: 'gamepad',
    });
    assert.equal(context.os, 'unknown');
    assert.deepEqual({ ...context.viewport }, { width: 0, height: 0 });
    assert.equal(context.safeArea.top, 0);
    assert.equal(context.colorScheme, 'unknown');
    assert.equal(context.reducedMotion, false);
    assert.equal(context.inputMode, 'unknown');
  });
}

for (const family of families.filter((name) => name !== 'react-dom')) {
  test(`${family}: frame loop clamps resume delta and releases owned resources`, () => {
    const env = browser();
    const deltas = [];
    const { createFrameLoop } = load(family, 'frame-loop.js', env);
    const loop = createFrameLoop(env.canvas, { onFrame: (frame) => deltas.push(frame.dt) });
    loop.start();
    loop.start();
    assert.equal(env.frames.size, 1);
    assert.equal(env.canvas.width, 200);
    env.tick(100);
    env.tick(5100);
    assert.equal(deltas[0], 0);
    assert.equal(deltas[1], 1 / 15);
    env.document.emit('visibilitychange');
    env.tick(10100);
    assert.equal(deltas[2], 0);
    loop.stop();
    loop.stop();
    assert.equal(env.frames.size, 0);
    assert.equal(env.window.count(), 0);
    assert.equal(env.document.count(), 0);
    assert.equal(env.observer.disconnected, true);
  });
}

for (const family of families.filter((name) => name !== 'react-dom')) {
  test(`${family}: DPR changes resize backing store even with ResizeObserver available`, () => {
    const env = browser();
    const { createFrameLoop } = load(family, 'frame-loop.js', env);
    const loop = createFrameLoop(env.canvas);
    loop.start();
    env.window.devicePixelRatio = 3;
    env.window.emit('resize');
    assert.equal(env.canvas.width, 300);
    env.window.devicePixelRatio = 1;
    env.document.emit('visibilitychange');
    assert.equal(env.canvas.width, 100);
    loop.stop();
    assert.equal(env.window.count(), 0);
  });
}

function reactHarness() {
  const states = [];
  const effects = [];
  let stateIndex = 0;
  let effectIndex = 0;
  let pending = [];
  const hooks = {
    createContext: () => ({}),
    useContext: () => null,
    useMemo: (fn) => fn(),
    useState(init) {
      const index = stateIndex++;
      if (!(index in states)) states[index] = typeof init === 'function' ? init() : init;
      return [states[index], (update) => { states[index] = typeof update === 'function' ? update(states[index]) : update; }];
    },
    useEffect(fn, dependencies) {
      const index = effectIndex++;
      if (!effects[index] || dependencies.some((value, i) => !Object.is(value, effects[index].dependencies[i]))) {
        pending.push(() => { effects[index]?.cleanup?.(); effects[index] = { dependencies, cleanup: fn() }; });
      }
    },
  };
  return { hooks,
    render(component) {
      stateIndex = 0; effectIndex = 0;
      const value = component({ children: null });
      const run = pending; pending = []; for (const effect of run) effect();
      return value;
    },
    cleanup() { for (const effect of effects) effect?.cleanup?.(); effects.length = 0; },
  };
}

for (const family of families) {
  test(`${family}: provider follows resume, viewport and appearance changes; StrictMode cleans listeners`, () => {
    const env = browser();
    const media = mediaQueries();
    env.window.matchMedia = media.matchMedia;
    env.window.innerWidth = 320;
    env.window.innerHeight = 600;
    env.window.screen = { width: 390, height: 844 };
    const toggles = [];
    env.document.documentElement = {
      dataset: {}, style: { setProperty() {} },
      classList: { toggle(name, on) { toggles.push([name, on]); } },
    };
    const globals = { ...env, navigator: { userAgent: IPHONE, maxTouchPoints: 5 } };
    const deviceModule = load(family, 'device-context.js', globals);
    const platform = load(family, 'platform-adapter.js', { ...globals, ...deviceModule });
    const react = reactHarness();
    const { PlatformProvider } = load(family, 'platform-provider.jsx', {
      ...globals, ...deviceModule, ...platform, ...react.hooks,
    });

    assert.equal(react.render(PlatformProvider).device.viewport.width, 320);
    // Published on <html>, where no app screen can drop it.
    assert.equal(env.document.documentElement.dataset.platform, 'ios:iphone');
    env.window.innerWidth = 400;
    env.window.emit('pageshow');
    assert.equal(react.render(PlatformProvider).device.viewport.width, 400);
    env.window.innerWidth = 450;
    env.document.emit('visibilitychange');
    assert.equal(react.render(PlatformProvider).device.viewport.width, 450);
    env.window.innerWidth = 470;
    env.window.visualViewport.emit('scroll');
    assert.equal(react.render(PlatformProvider).device.viewport.width, 470);

    const dark = media.lists.get('(prefers-color-scheme: dark)');
    dark.matches = true;
    dark.emit('change');
    assert.equal(react.render(PlatformProvider).device.colorScheme, 'dark');
    assert.deepEqual(toggles.at(-1), ['ion-palette-dark', true]);

    react.cleanup();
    assert.equal(env.window.count() + env.window.visualViewport.count() + env.document.count(), 0);
    assert.equal([...media.lists.values()].reduce((sum, item) => sum + item.count(), 0), 0);
    react.render(PlatformProvider);
    env.window.innerWidth = 500;
    env.window.emit('resize');
    assert.equal(react.render(PlatformProvider).device.viewport.width, 500);
    react.cleanup();
  });
}

function runtimeLoop() {
  const state = { starts: 0, stops: 0 };
  return { state, createFrameLoop: () => ({ start() { state.starts += 1; }, stop() { state.stops += 1; }, size: { width: 100, height: 80 } }) };
}

test('Phaser: disposal between READY and its microtask destroys once; terminal stop cannot restart', async () => {
  const scheduling = runtimeLoop();
  let game;
  class Game {
    constructor() {
      game = this;
      this.isBooted = true;
      this.isRunning = true;
      this.events = { once: (_event, fn) => { this.ready = fn; } };
      this.loop = { stop() {} };
      this.destroyCount = 0;
    }
    destroy() { this.destroyCount += 1; }
    step() {}
  }
  const { createPhaserRuntime } = load('phaser-2d', 'phaser-runtime.js', {
    Phaser: { Game, CANVAS: 1, Scale: { NONE: 0 } }, ...scheduling, queueMicrotask, performance,
  });
  const runtime = createPhaserRuntime(browser().canvas);
  runtime.start();
  game.ready();
  runtime.stop();
  await Promise.resolve();
  runtime.start();
  runtime.stop();
  assert.equal(game.destroyCount, 1);
  assert.equal(scheduling.state.starts, 1);
  assert.equal(scheduling.state.stops, 1);
});

test('Phaser: stop before asynchronous boot releases the eventual game exactly once', async () => {
  let game;
  class Game {
    constructor() {
      game = this; this.isBooted = false; this.isRunning = false; this.destroyCount = 0;
      this.events = { once: (_event, fn) => { this.ready = fn; } };
      this.loop = { stop() {} };
    }
    destroy() { this.destroyCount += 1; }
    step() {}
  }
  const scheduling = runtimeLoop();
  const { createPhaserRuntime } = load('phaser-2d', 'phaser-runtime.js', {
    Phaser: { Game, CANVAS: 1, Scale: { NONE: 0 } }, ...scheduling, queueMicrotask, performance,
  });
  const runtime = createPhaserRuntime(browser().canvas);
  runtime.stop();
  game.isBooted = true; game.isRunning = true;
  game.ready();
  await Promise.resolve();
  assert.equal(game.destroyCount, 1);
  runtime.start();
  assert.equal(scheduling.state.starts, 0);
});

test('Babylon: terminal cleanup prevents restarting a disposed engine and late physics initialization', async () => {
  const scheduling = runtimeLoop();
  let disposed = 0;
  let physicsEnabled = 0;
  let resolveHavok;
  class Engine { dispose() { disposed += 1; } }
  class Scene { dispose() { disposed += 1; } enablePhysics() { physicsEnabled += 1; return true; } }
  class Vector3 { static Zero() { return new Vector3(); } }
  class ArcRotateCamera { attachControl() {} }
  class Value {}
  const { createBabylonRuntime } = load('babylon-3d', 'babylon-runtime.js', {
    ...scheduling, Engine, Scene, Vector3, ArcRotateCamera,
    Color3: Value, Color4: Value, HemisphericLight: Value, StandardMaterial: Value,
    HavokPlugin: Value, PhysicsAggregate: Value, PhysicsShapeType: { BOX: 0 },
    MeshBuilder: { CreateBox: () => ({ position: {} }) },
    HavokPhysics: () => new Promise((resolve) => { resolveHavok = resolve; }),
  });
  const runtime = createBabylonRuntime(browser().canvas);
  runtime.start(); runtime.stop(); runtime.start(); runtime.stop();
  resolveHavok({});
  await Promise.resolve();
  assert.equal(physicsEnabled, 0);
  assert.equal(disposed, 2);
  assert.equal(scheduling.state.starts, 1);
  assert.equal(scheduling.state.stops, 1);
});
