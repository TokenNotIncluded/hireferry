// SPDX-License-Identifier: AGPL-3.0-or-later
import { mountSculpture } from './runtime/motion.js';
import { createTimeline } from './runtime/timeline.js';

/** Sample the browser's own glyph positions, including wrapping and CJK fonts. */
function sampleHeading(heading) {
  const bounds = heading.getBoundingClientRect();
  if (!bounds.width || !bounds.height) return null;
  const mask = document.createElement('canvas');
  mask.width = Math.ceil(bounds.width);
  mask.height = Math.ceil(bounds.height);
  const context = mask.getContext('2d', { willReadFrequently: true });
  if (!context) return null;
  const walker = document.createTreeWalker(heading, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  let node;
  while ((node = walker.nextNode())) {
    if (node.parentElement.closest('canvas')) continue;
    const style = getComputedStyle(node.parentElement);
    context.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
    context.fillStyle = node.parentElement.closest('em') ? '#ffffff' : '#400000';
    let offset = 0;
    for (const character of node.textContent) {
      range.setStart(node, offset);
      range.setEnd(node, offset + character.length);
      offset += character.length;
      const rect = range.getBoundingClientRect();
      if (!rect.width || !character.trim()) continue;
      const metrics = context.measureText(character);
      const ascent = metrics.fontBoundingBoxAscent ?? Number.parseFloat(style.fontSize) * .8;
      const descent = metrics.fontBoundingBoxDescent ?? Number.parseFloat(style.fontSize) * .2;
      const baseline = rect.top - bounds.top + (rect.height - ascent - descent) / 2 + ascent;
      context.fillText(character, rect.left - bounds.left, baseline);
    }
  }
  const { data } = context.getImageData(0, 0, mask.width, mask.height);
  const grid = Math.max(1.6, Math.min(2.5, Number.parseFloat(getComputedStyle(heading).fontSize) / 28));
  const points = [];
  for (let y = 0; y < mask.height; y += grid) for (let x = 0; x < mask.width; x += grid) {
    const slot = (Math.floor(y) * mask.width + Math.floor(x)) * 4;
    if (data[slot + 3] < 110) continue;
    points.push({ x: x / bounds.width, y: y / bounds.height, z: 0,
      color: [32, 32, 32], material: data[slot] > 200 ? 'primary' : 'ink' });
  }
  if (!points.length) return null;
  return { grid, model: {
    id: 'type', points, view: [0, 0],
    animate: time => (point, out) => {
      out[0] += Math.sin(time * .7 + point.y * 8) * .6 / bounds.width;
      out[1] += Math.sin(time * .9 + point.x * 5) * .35 / bounds.height;
    },
  } };
}

/** Real headings remain selectable and accessible; canvases are decorative. */
export function mountParticleTitles(section) {
  const titles = [...section.querySelectorAll('.hero-slide h1')];
  let disposed = false, position = 0, paused = navigator.connection?.saveData === true;
  const entries = titles.map((heading, index) => {
    heading.classList.add('particle-title');
    const canvas = document.createElement('canvas');
    canvas.className = 'particle-title-canvas';
    canvas.setAttribute('aria-hidden', 'true');
    heading.append(canvas);
    const entry = { heading, canvas, motion: null, width: 0, height: 0 };
    function rebuild(force = false) {
      if (disposed || !heading.isConnected) return;
      const { width, height } = heading.getBoundingClientRect();
      if (!force && width === entry.width && height === entry.height) return;
      entry.width = width; entry.height = height;
      entry.motion?.dispose();
      entry.motion = null;
      heading.classList.remove('particle-title-ready');
      try {
        const sample = sampleHeading(heading);
        if (!sample) return;
        entry.motion = mountSculpture(canvas, createTimeline([sample.model]), {
          pointerTarget: heading,
          suspended: Math.abs(index - position) >= 1,
          rendererOptions: { flat: true, grid: sample.grid, maxPoints: 9000 },
          onReady() { if (!disposed && heading.isConnected) heading.classList.add('particle-title-ready'); },
        });
        entry.motion?.pause(paused);
      } catch {
        // Canvas/font access may be restricted. The original heading remains visible.
        entry.motion?.dispose(); entry.motion = null;
        canvas.width = canvas.height = 0;
      }
    }
    entry.resize = new ResizeObserver(() => rebuild());
    entry.resize.observe(heading);
    rebuild();
    document.fonts?.ready.then(() => rebuild(true)).catch(() => {});
    return entry;
  });
  return {
    seek(value) {
      position = value;
      entries.forEach((entry, index) => entry.motion?.suspend(Math.abs(index - position) >= 1));
    },
    pause(value = !paused) { paused = Boolean(value); entries.forEach(entry => entry.motion?.pause(paused)); },
    dispose() {
      disposed = true;
      entries.forEach(({ heading, canvas, motion, resize }) => {
        resize.disconnect(); motion?.dispose(); canvas.remove();
        heading.classList.remove('particle-title', 'particle-title-ready');
      });
    },
  };
}
