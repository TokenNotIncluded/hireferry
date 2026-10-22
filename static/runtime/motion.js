/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { createRenderer } from './renderer.js';

/** Own exactly one RAF. Pointer coordinates and velocity are in CSS pixels. */
export function mountSculpture(canvas, timeline, { onFrame = () => {}, onState = () => {}, onReady = () => {}, rendererOptions = {}, pointerTarget = canvas, suspended: initiallySuspended = false } = {}) {
  const renderer = createRenderer(canvas, timeline, onReady, rendererOptions);
  if (!renderer) { onState('unsupported'); return null; }
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');
  const fine = matchMedia('(hover: hover) and (pointer: fine)');
  const pointer = { x: 0, y: 0, previousX: 0, previousY: 0, vx: 0, vy: 0, active: false };
  let target = { x: 0, y: 0, active: false };
  let frame = null, disposed = false, visible = true, dirty = true, suspended = initiallySuspended;
  let paused = navigator.connection?.saveData === true, last = 0, time = 0, selected = null;
  const state = () => reduced.matches ? 'reduced' : paused ? 'paused' : 'playing';
  let announced;
  const announce = () => {
    const current = state();
    if (current !== announced) { announced = current; onState(current); }
  };
  const schedule = () => {
    if (frame === null && !disposed && visible && !suspended && !document.hidden) frame = requestAnimationFrame(render);
  };
  const refresh = () => { dirty = true; schedule(); };
  const resetClock = () => {
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null; last = 0; target.active = false;
    announce(); refresh();
  };
  function render(now) {
    frame = null;
    if (disposed || document.hidden || !visible || suspended) return;
    announce();
    const budget = fine.matches && innerWidth > 900 ? 1000 / 60 : 1000 / 30;
    const elapsed = last ? Math.min(80, now - last) : budget;
    if (!dirty && elapsed + 0.5 < budget) { schedule(); return; }
    last = now;
    const animate = !paused && !reduced.matches;
    const dt = elapsed / 1000;
    if (animate) time += dt;
    pointer.previousX = pointer.x; pointer.previousY = pointer.y;
    pointer.x += (target.x - pointer.x) * (1 - Math.exp(-elapsed / 75));
    pointer.y += (target.y - pointer.y) * (1 - Math.exp(-elapsed / 75));
    const vx = Math.max(-1600, Math.min(1600, (pointer.x - pointer.previousX) / Math.max(dt, .001)));
    const vy = Math.max(-1600, Math.min(1600, (pointer.y - pointer.previousY) / Math.max(dt, .001)));
    pointer.vx += (vx - pointer.vx) * (1 - Math.exp(-elapsed / 110));
    pointer.vy += (vy - pointer.vy) * (1 - Math.exp(-elapsed / 110));
    pointer.active = target.active && animate;
    renderer.draw(time, pointer, dt, !animate, selected);
    onFrame(renderer.inspect()); dirty = false;
    if (animate) schedule();
  }
  const brush = (x, y) => {
    if (paused || reduced.matches) return;
    const rect = canvas.getBoundingClientRect();
    if (!target.active) {
      pointer.x = pointer.previousX = x - rect.left;
      pointer.y = pointer.previousY = y - rect.top;
      pointer.vx = pointer.vy = 0;
    }
    target = { x: x - rect.left, y: y - rect.top, active: true }; schedule();
  };
  const move = event => brush(event.clientX, event.clientY);
  const leave = () => { target.active = false; schedule(); };
  const touch = event => {
    if (event.touches.length === 1) brush(event.touches[0].clientX, event.touches[0].clientY);
    else leave();
  };
  const resize = new ResizeObserver(refresh);
  const intersection = new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting; resetClock();
  });
  const theme = new MutationObserver(refresh);
  resize.observe(canvas); intersection.observe(canvas);
  theme.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'data-theme', 'data-palette'] });
  pointerTarget.addEventListener('pointermove', move, { passive: true });
  pointerTarget.addEventListener('pointerdown', move, { passive: true });
  for (const name of ['pointerleave', 'pointercancel', 'pointerup']) pointerTarget.addEventListener(name, leave);
  pointerTarget.addEventListener('touchstart', touch, { passive: true });
  pointerTarget.addEventListener('touchmove', touch, { passive: true });
  pointerTarget.addEventListener('touchend', leave, { passive: true });
  pointerTarget.addEventListener('touchcancel', leave, { passive: true });
  document.addEventListener('visibilitychange', resetClock);
  reduced.addEventListener('change', resetClock);
  announce(); schedule();
  return {
    select(index) { selected = index; time = 0; resetClock(); },
    seek(position) { selected = position; refresh(); },
    pause(value = !paused) { paused = Boolean(value); resetClock(); },
    suspend(value) { if (suspended !== Boolean(value)) { suspended = Boolean(value); resetClock(); } },
    inspect() { return { ...renderer.inspect(), state: state(), visible, selected }; },
    dispose() {
      disposed = true; if (frame !== null) cancelAnimationFrame(frame);
      resize.disconnect(); intersection.disconnect(); theme.disconnect();
      pointerTarget.removeEventListener('pointermove', move); pointerTarget.removeEventListener('pointerdown', move);
      for (const name of ['pointerleave', 'pointercancel', 'pointerup']) pointerTarget.removeEventListener(name, leave);
      pointerTarget.removeEventListener('touchstart', touch); pointerTarget.removeEventListener('touchmove', touch);
      pointerTarget.removeEventListener('touchend', leave); pointerTarget.removeEventListener('touchcancel', leave);
      document.removeEventListener('visibilitychange', resetClock);
      reduced.removeEventListener('change', resetClock); renderer.dispose();
    },
  };
}
