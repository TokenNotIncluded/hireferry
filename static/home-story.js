// SPDX-License-Identifier: AGPL-3.0-or-later
const clamp = (value, low, high) => Math.max(low, Math.min(high, value));

// Each chapter settles before the next one starts; the final chapter has room
// to breathe before the sticky element naturally leaves with the document.
export function storyPosition(offset, travel, count) {
  const track = clamp(offset / Math.max(1, travel), 0, 1) * (count - .6);
  const chapter = Math.floor(track);
  const phase = clamp((track - chapter) / .68, 0, 1);
  const eased = phase * phase * (3 - 2 * phase);
  return clamp(chapter + eased, 0, count - 1);
}

/** Native scrolling owns touch momentum, zoom, keyboard scrolling and escape.
 * The spring only follows the document; it never cancels a wheel/touch event. */
export function mountHomeStory(section, { onPosition }) {
  const hero = section.querySelector('.hero');
  const slides = [...section.querySelectorAll('[data-story-slide]')];
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');
  let start = 0, travel = 1, target = 0, position = 0, velocity = 0;
  let frame = null, previous = 0, active = -1, disposed = false, scrolling = false;

  function paint() {
    const current = Math.round(position);
    slides.forEach((slide, index) => {
      const distance = index - position;
      slide.style.opacity = String(Math.max(0, 1 - Math.abs(distance) * 1.35));
      slide.style.transform = `translate3d(0, ${scrolling ? distance * 56 : 0}px, 0)`;
    });
    onPosition(position);
    if (current === active) return;
    active = current;
    section.dataset.chapter = String(current);
    slides.forEach((slide, index) => slide.setAttribute('aria-hidden', String(index !== current)));
  }
  function render(now) {
    frame = null;
    if (disposed || !scrolling) return;
    const dt = Math.min((now - (previous || now - 16)) / 1000, .032);
    previous = now;
    // Analytic critically damped spring: retargets from the live position and
    // velocity even during a rapid scroll reversal, independent of frame rate.
    const omega = 22, displacement = position - target;
    const impulse = velocity + omega * displacement, decay = Math.exp(-omega * dt);
    position = target + (displacement + impulse * dt) * decay;
    velocity = (velocity - omega * impulse * dt) * decay;
    if (Math.abs(target - position) < .0005 && Math.abs(velocity) < .005) {
      position = target; velocity = 0;
    }
    paint();
    if (position !== target || velocity !== 0) schedule();
    else previous = 0;
  }
  function schedule() { if (frame === null && !disposed) frame = requestAnimationFrame(render); }
  function update() {
    if (disposed || !scrolling) return;
    target = storyPosition(scrollY - start, travel, slides.length);
    if (active === -1) { position = target; paint(); }
    else if (position !== target || velocity !== 0) schedule();
  }
  function measure() {
    if (disposed) return;
    const style = getComputedStyle(hero), inset = Number.parseFloat(style.top);
    travel = section.offsetHeight - hero.offsetHeight;
    // Mobile and reduced-motion layouts use normal document flow. They have
    // no sticky travel; keep the first chapter readable without a scroll loop.
    const enabled = !reduced.matches && style.position === 'sticky'
      && Number.isFinite(inset) && travel > 1 && slides.length > 1;
    if (enabled !== scrolling) {
      scrolling = enabled;
      if (scrolling) window.addEventListener('scroll', update, { passive: true });
      else window.removeEventListener('scroll', update);
    }
    if (!scrolling) {
      if (frame !== null) cancelAnimationFrame(frame);
      frame = null; previous = 0; velocity = 0; target = 0; position = 0;
      paint();
      return;
    }
    start = section.getBoundingClientRect().top + scrollY - inset;
    update();
  }
  const resize = new ResizeObserver(measure);
  resize.observe(section); resize.observe(hero);
  window.addEventListener('resize', measure, { passive: true });
  reduced.addEventListener('change', measure);
  measure();
  return {
    dispose() {
      disposed = true;
      if (frame !== null) cancelAnimationFrame(frame);
      resize.disconnect();
      window.removeEventListener('scroll', update);
      window.removeEventListener('resize', measure);
      reduced.removeEventListener('change', measure);
    },
  };
}
