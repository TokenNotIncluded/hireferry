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
export function mountHomeStory(section, { onPosition, captions }) {
  const hero = section.querySelector('.hero');
  const slides = [...section.querySelectorAll('[data-story-slide]')];
  const buttons = [...section.querySelectorAll('[data-scene]')];
  const caption = section.querySelector('#scene-caption');
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');
  let start = 0, travel = 1, target = 0, position = 0, velocity = 0;
  let frame = null, previous = 0, active = -1, disposed = false;

  function paint() {
    const current = Math.round(position);
    slides.forEach((slide, index) => {
      const distance = index - position;
      slide.style.opacity = String(Math.max(0, 1 - Math.abs(distance) * 1.35));
      slide.style.transform = `translate3d(0, ${reduced.matches ? 0 : distance * 56}px, 0)`;
    });
    onPosition(position);
    if (current === active) return;
    active = current;
    section.dataset.chapter = String(current);
    slides.forEach((slide, index) => slide.setAttribute('aria-hidden', String(index !== current)));
    buttons.forEach((button, index) => button.setAttribute('aria-pressed', String(index === current)));
    caption.textContent = captions[current];
  }
  function render(now) {
    frame = null;
    if (disposed) return;
    const dt = Math.min((now - (previous || now - 16)) / 1000, .032);
    previous = now;
    if (reduced.matches) { position = Math.round(target); velocity = 0; }
    else {
      // Analytic critically damped spring: retargets from the live position and
      // velocity even during a rapid scroll reversal, independent of frame rate.
      const omega = 22, displacement = position - target;
      const impulse = velocity + omega * displacement, decay = Math.exp(-omega * dt);
      position = target + (displacement + impulse * dt) * decay;
      velocity = (velocity - omega * impulse * dt) * decay;
      if (Math.abs(target - position) < .0005 && Math.abs(velocity) < .005) {
        position = target; velocity = 0;
      }
    }
    paint();
    if (position !== target && !reduced.matches) schedule();
    else previous = 0;
  }
  function schedule() { if (frame === null && !disposed) frame = requestAnimationFrame(render); }
  function update() {
    target = storyPosition(scrollY - start, travel, slides.length);
    schedule();
  }
  function measure() {
    start = section.getBoundingClientRect().top + scrollY - Number.parseFloat(getComputedStyle(hero).top);
    travel = Math.max(1, section.offsetHeight - hero.offsetHeight);
    update();
  }
  function select(index) {
    const chapter = clamp(index, 0, slides.length - 1);
    const offset = chapter === 0 ? 0 : (chapter - .12) / (slides.length - .6) * travel;
    window.scrollTo({ top: start + offset, behavior: reduced.matches ? 'instant' : 'smooth' });
  }
  function keydown(event) {
    if (!['ArrowLeft', 'ArrowRight'].includes(event.key) || event.altKey || event.ctrlKey || event.metaKey) return;
    if (!event.target.closest('[data-scene]')) return;
    event.preventDefault();
    const next = clamp(active + (event.key === 'ArrowRight' ? 1 : -1), 0, slides.length - 1);
    buttons[next].focus({ preventScroll: true });
    select(next);
  }
  const resize = new ResizeObserver(measure);
  resize.observe(section); resize.observe(hero);
  window.addEventListener('scroll', update, { passive: true });
  window.addEventListener('resize', measure, { passive: true });
  reduced.addEventListener('change', update);
  section.addEventListener('keydown', keydown);
  measure(); position = reduced.matches ? Math.round(target) : target; paint();
  return {
    select,
    dispose() {
      disposed = true;
      if (frame !== null) cancelAnimationFrame(frame);
      resize.disconnect();
      window.removeEventListener('scroll', update);
      window.removeEventListener('resize', measure);
      reduced.removeEventListener('change', update);
      section.removeEventListener('keydown', keydown);
    },
  };
}
