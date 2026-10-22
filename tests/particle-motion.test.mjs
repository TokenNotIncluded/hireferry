import test from 'node:test';
import assert from 'node:assert/strict';
import { mountSculpture } from '../static/runtime/motion.js';
import { createTimeline } from '../static/runtime/timeline.js';
import { mountBrandMotion } from '../static/brand-motion.js';

test('particle owners stop frames when paused, reduced, suspended, hidden, or disposed', () => {
  const originals = new Map();
  const install = (name, value) => {
    originals.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { configurable: true, value });
  };
  const frames = new Map();
  const observers = { resize: [], intersection: [], theme: [] };
  const observer = kind => class {
    constructor(callback) { this.callback = callback; this.disconnected = false; observers[kind].push(this); }
    observe() {}
    disconnect() { this.disconnected = true; }
  };
  const reduced = Object.assign(new EventTarget(), { matches: false });
  const fine = Object.assign(new EventTarget(), { matches: true });
  const document = Object.assign(new EventTarget(), {
    hidden: false, documentElement: { className: 'light', dataset: { theme: 'light', palette: 'neutral' } },
  });
  let id = 0, now = 0, painted = null;
  const canvas = Object.assign(new EventTarget(), {
    clientWidth: 80, clientHeight: 40, width: 0, height: 0, dataset: {},
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
    getContext: () => ({
      createImageData: (width, height) => ({ data: new Uint8ClampedArray(width * height * 4) }),
      putImageData: image => { painted = image.data.slice(); },
    }),
  });
  const tick = () => {
    const callbacks = [...frames.values()]; frames.clear(); now += 40;
    callbacks.forEach(callback => callback(now));
  };
  let motion;
  try {
    install('document', document); install('window', { devicePixelRatio: 1 });
    install('navigator', {}); install('innerWidth', 1000);
    install('matchMedia', query => query.includes('reduced') ? reduced : fine);
    install('requestAnimationFrame', callback => { frames.set(++id, callback); return id; });
    install('cancelAnimationFrame', handle => frames.delete(handle));
    install('ResizeObserver', observer('resize'));
    install('IntersectionObserver', observer('intersection'));
    install('MutationObserver', observer('theme'));
    install('getComputedStyle', () => ({ getPropertyValue: role => role === '--poster-ground' ? '#f9f8f4'
      : document.documentElement.dataset.theme === 'dark' ? '#f2eee5' : '#252420' }));
    const model = { id: 'type', view: [0, 0], animate: () => () => {},
      points: [{ x: .4, y: .4, z: 0, color: [32, 32, 32], material: 'ink' }] };
    motion = mountSculpture(canvas, createTimeline([model]), { rendererOptions: { flat: true, maxPoints: 1 } });
    tick(); assert.equal(motion.inspect().frames, 1); assert.equal(frames.size, 1);

    motion.pause(true); tick(); assert.equal(frames.size, 0);
    const still = painted, time = motion.inspect().time, count = motion.inspect().frames;
    document.documentElement.dataset.theme = 'dark'; observers.theme[0].callback(); tick();
    assert.equal(motion.inspect().frames, count + 1);
    assert.equal(motion.inspect().time, time); assert.equal(frames.size, 0);
    assert.notDeepEqual(painted, still, 'paused text repaints using the new theme');

    motion.pause(false); tick(); assert.equal(frames.size, 1);
    motion.suspend(true); assert.equal(frames.size, 0);
    observers.theme[0].callback(); assert.equal(frames.size, 0);
    motion.suspend(false); tick(); assert.equal(frames.size, 1);

    reduced.matches = true; reduced.dispatchEvent(new Event('change')); tick();
    assert.equal(motion.inspect().state, 'reduced'); assert.equal(frames.size, 0);
    reduced.matches = false; reduced.dispatchEvent(new Event('change')); tick();
    assert.equal(frames.size, 1);

    document.hidden = true; document.dispatchEvent(new Event('visibilitychange'));
    assert.equal(frames.size, 0);
    document.hidden = false; document.dispatchEvent(new Event('visibilitychange')); tick();
    observers.intersection[0].callback([{ isIntersecting: false }]); assert.equal(frames.size, 0);
    observers.intersection[0].callback([{ isIntersecting: true }]); tick(); assert.equal(frames.size, 1);

    motion.dispose(); assert.equal(frames.size, 0);
    assert.equal(canvas.width, 0); assert.equal(canvas.height, 0);
    assert.ok(Object.values(observers).flat().every(item => item.disconnected));
    reduced.dispatchEvent(new Event('change')); document.dispatchEvent(new Event('visibilitychange'));
    assert.equal(frames.size, 0, 'disposed owners cannot schedule more frames');
  } finally {
    motion?.dispose();
    for (const [name, descriptor] of originals) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete globalThis[name];
    }
  }
});

test('retained navigation restores logo visibility without waiting for a new intersection', () => {
  const originals = new Map();
  const install = (name, value) => {
    originals.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { configurable: true, value });
  };
  let intersection, mutation, observed, writes = 0, motion;
  const brand = { isConnected: true, style: { removeProperty() {} },
    dataset: new Proxy({}, { set(target, key, value) { writes++; target[key] = value; return true; } }) };
  const document = Object.assign(new EventTarget(), {
    hidden: false, body: {}, documentElement: { dataset: {} }, querySelectorAll: () => [brand],
  });
  const media = Object.assign(new EventTarget(), { matches: false });
  try {
    install('document', document); install('matchMedia', () => media);
    install('IntersectionObserver', class {
      constructor(callback) { intersection = callback; }
      observe() {} unobserve() {} disconnect() {}
    });
    install('MutationObserver', class {
      constructor(callback) { mutation = callback; }
      observe(_target, options) { observed = options; } disconnect() {}
    });
    motion = mountBrandMotion();
    intersection([{ target: brand, isIntersecting: true }]);
    assert.equal(brand.dataset.brandVisible, 'true');
    delete brand.dataset.brandVisible; // The page reconciler strips unrendered attributes.
    mutation([{ type: 'attributes', target: brand, attributeName: 'data-brand-visible' }]);
    assert.equal(brand.dataset.brandVisible, 'true');
    const repaired = writes; mutation([]); assert.equal(writes, repaired, 'self-observation does not loop');
    assert.deepEqual(observed.attributeFilter, ['data-brand-visible']);
    media.matches = true; media.dispatchEvent(new Event('change'));
    assert.equal(document.documentElement.dataset.brandMotion, 'paused');
    motion.dispose(); assert.equal(brand.dataset.brandVisible, undefined);
    mutation([]); assert.equal(brand.dataset.brandVisible, undefined);
  } finally {
    motion?.dispose();
    for (const [name, descriptor] of originals) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete globalThis[name];
    }
  }
});
