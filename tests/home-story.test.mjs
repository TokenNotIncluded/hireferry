import test from 'node:test';
import assert from 'node:assert/strict';
import { mountHomeStory, storyPosition } from '../static/home-story.js';
import { createTimeline } from '../static/runtime/timeline.js';

function mountFixture(t, { position = 'sticky', top = '100px', travel = 2400, reduced = false, scroll = 0 } = {}) {
  const descriptors = new Map(), listeners = new Map(), mediaListeners = new Map(), frames = new Map();
  const positions = [], layout = { position, top }, media = { matches: reduced };
  let nextFrame = 0, now = 0, scrollPosition = scroll, observer, view;
  const listen = (map, type, callback) => {
    if (!map.has(type)) map.set(type, new Set());
    map.get(type).add(callback);
  };
  const remove = (map, type, callback) => map.get(type)?.delete(callback);
  const emit = (map, type) => [...(map.get(type) || [])].forEach(callback => callback());
  const install = (name, value) => {
    descriptors.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { configurable: true, value });
  };
  const slides = Array.from({ length: 3 }, () => ({
    style: {}, attributes: {}, setAttribute(name, value) { this.attributes[name] = value; },
  }));
  const hero = { offsetHeight: 600 };
  const section = {
    dataset: {}, offsetHeight: 600 + travel,
    querySelector: () => hero, querySelectorAll: () => slides,
    getBoundingClientRect: () => ({ top: 100 - scrollPosition }),
  };
  t.after(() => {
    view?.dispose();
    for (const [name, descriptor] of descriptors) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete globalThis[name];
    }
  });
  install('window', {
    addEventListener: (type, callback) => listen(listeners, type, callback),
    removeEventListener: (type, callback) => remove(listeners, type, callback),
  });
  install('scrollY', scrollPosition);
  Object.defineProperty(globalThis, 'scrollY', { configurable: true, get: () => scrollPosition });
  media.addEventListener = (type, callback) => listen(mediaListeners, type, callback);
  media.removeEventListener = (type, callback) => remove(mediaListeners, type, callback);
  install('matchMedia', () => media);
  install('getComputedStyle', () => layout);
  install('ResizeObserver', class {
    constructor(callback) { this.callback = callback; this.disconnected = false; observer = this; }
    observe() {}
    disconnect() { this.disconnected = true; }
  });
  install('requestAnimationFrame', callback => { const id = ++nextFrame; frames.set(id, callback); return id; });
  install('cancelAnimationFrame', id => frames.delete(id));
  view = mountHomeStory(section, { onPosition: value => positions.push(value) });
  return {
    slides, section, positions, frames, layout, listeners, mediaListeners,
    get observer() { return observer; },
    scroll(value) { scrollPosition = value; emit(listeners, 'scroll'); },
    resize({ position, top, travel } = {}) {
      if (position !== undefined) layout.position = position;
      if (top !== undefined) layout.top = top;
      if (travel !== undefined) section.offsetHeight = hero.offsetHeight + travel;
      observer.callback();
    },
    reduce(value) { media.matches = value; emit(mediaListeners, 'change'); },
    tick() {
      const callbacks = [...frames.values()]; frames.clear(); now += 16;
      callbacks.forEach(callback => callback(now));
    },
    settle() {
      let count = 0;
      while (frames.size && count++ < 240) this.tick();
      assert.equal(frames.size, 0, 'animation must release its frame loop after settling');
      assert.ok(positions.every(Number.isFinite), 'chapter positions must stay finite');
    },
    dispose() { view.dispose(); },
  };
}

function assertFirstChapter(fixture) {
  assert.equal(fixture.section.dataset.chapter, '0');
  assert.deepEqual(fixture.slides.map(slide => slide.attributes['aria-hidden']), ['false', 'true', 'true']);
  assert.deepEqual(fixture.slides.map(slide => slide.style.opacity), ['1', '0', '0']);
  assert.ok(fixture.positions.every(value => value === 0));
  assert.equal(fixture.frames.size, 0);
}

test('scrolling visits all chapters once and holds the last before entering the page flow', () => {
  assert.equal(storyPosition(-400, 2400, 3), 0);
  assert.equal(storyPosition(0, 2400, 3), 0);
  assert.equal(storyPosition(800, 2400, 3), 1);
  assert.equal(storyPosition(1800, 2400, 3), 2);
  assert.equal(storyPosition(2400, 2400, 3), 2);
  assert.equal(storyPosition(4000, 2400, 3), 2);
});

test('slow scrolling and reversing keep progress continuous and never wrap to the first chapter', () => {
  let previous = 0;
  for (let offset = 0; offset <= 2700; offset++) {
    const position = storyPosition(offset, 2400, 3);
    assert.ok(position >= previous && position <= 2);
    assert.ok(position - previous < .003);
    previous = position;
  }
  const forward = [0, 350, 800, 1350, 1800, 2400].map(y => storyPosition(y, 2400, 3));
  const backward = [2400, 1800, 1350, 800, 350, 0].map(y => storyPosition(y, 2400, 3));
  assert.deepEqual(backward, forward.toReversed());
});

test('particle transitions follow fractional chapter progress and keep ambient time on reversal', () => {
  const scenes = ['ferry', 'bridge', 'compass'].map(id => ({ id }));
  const timeline = createTimeline(scenes);
  const forward = timeline.at(17, .5);
  assert.equal(forward.from.id, 'ferry');
  assert.equal(forward.to.id, 'bridge');
  assert.equal(forward.mix, .5);
  assert.equal(forward.age, 17);
  assert.equal(forward.nextAge, 17);
  assert.deepEqual(timeline.at(17, .5), forward);
  assert.equal(timeline.at(17, 2).mix, 0);
  assert.equal(timeline.at(17, 100).from.id, 'compass');
  assert.equal(timeline.at(17, -4).from.id, 'ferry');
});

test('mobile normal-flow layout keeps its first chapter visible without animation work', t => {
  const fixture = mountFixture(t, { position: 'relative', top: '0px', travel: 0, scroll: 600 });
  assertFirstChapter(fixture);
  assert.equal(fixture.listeners.get('scroll')?.size || 0, 0);
  fixture.scroll(1800);
  fixture.resize();
  assertFirstChapter(fixture);
});

test('sticky layout requires a numeric inset and usable scroll travel', async t => {
  for (const layout of [{ top: 'auto' }, { travel: 0 }, { travel: 1 }]) {
    await t.test(JSON.stringify(layout), t => {
      const fixture = mountFixture(t, layout);
      fixture.scroll(900);
      assertFirstChapter(fixture);
    });
  }
});

test('desktop chapters settle, reverse and leave no frame scheduled at rest', t => {
  const fixture = mountFixture(t);
  assertFirstChapter(fixture);
  fixture.scroll(800);
  assert.equal(fixture.frames.size, 1);
  fixture.settle();
  assert.equal(fixture.positions.at(-1), 1);
  assert.deepEqual(fixture.slides.map(slide => slide.attributes['aria-hidden']), ['true', 'false', 'true']);
  const paints = fixture.positions.length;
  fixture.scroll(800);
  assert.equal(fixture.frames.size, 0);
  assert.equal(fixture.positions.length, paints);
  fixture.scroll(1800);
  fixture.settle();
  assert.equal(fixture.positions.at(-1), 2);
  fixture.scroll(0);
  fixture.settle();
  assert.equal(fixture.positions.at(-1), 0);
  assert.equal(fixture.slides[0].attributes['aria-hidden'], 'false');
});

test('restored desktop scroll position paints its current chapter immediately', t => {
  const fixture = mountFixture(t, { scroll: 1800 });
  assert.equal(fixture.positions.at(-1), 2);
  assert.equal(fixture.section.dataset.chapter, '2');
  assert.equal(fixture.frames.size, 0);
});

test('reduced motion keeps a readable first chapter and cancels an active transition', t => {
  const fixture = mountFixture(t, { reduced: true, scroll: 800 });
  assertFirstChapter(fixture);
  fixture.reduce(false);
  fixture.settle();
  assert.equal(fixture.positions.at(-1), 1);
  fixture.scroll(1800);
  fixture.tick();
  assert.equal(fixture.frames.size, 1);
  fixture.reduce(true);
  assert.equal(fixture.frames.size, 0);
  assert.equal(fixture.positions.at(-1), 0);
  assert.equal(fixture.slides[0].attributes['aria-hidden'], 'false');
  assert.equal(fixture.listeners.get('scroll')?.size || 0, 0);
  const paints = fixture.positions.length;
  fixture.scroll(2200);
  assert.equal(fixture.positions.length, paints);
});

test('resizing between desktop and mobile cancels and restores the scroll story', t => {
  const fixture = mountFixture(t);
  fixture.scroll(800);
  fixture.tick();
  fixture.resize({ position: 'relative', top: 'auto', travel: 0 });
  assert.equal(fixture.frames.size, 0);
  assert.equal(fixture.positions.at(-1), 0);
  assert.equal(fixture.slides[0].attributes['aria-hidden'], 'false');
  fixture.resize({ position: 'sticky', top: '100px', travel: 2400 });
  fixture.settle();
  assert.equal(fixture.positions.at(-1), 1);
  fixture.scroll(1800);
  assert.equal(fixture.frames.size, 1);
  fixture.dispose();
  assert.equal(fixture.frames.size, 0);
  assert.equal(fixture.listeners.get('scroll').size, 0);
  assert.equal(fixture.listeners.get('resize').size, 0);
  assert.equal(fixture.mediaListeners.get('change').size, 0);
  assert.equal(fixture.observer.disconnected, true);
});
