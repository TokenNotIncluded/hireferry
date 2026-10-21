/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { smooth } from './geometry.js';

/** Immutable models; time belongs to the motion owner, not the renderer. */
export function createTimeline(scenes, { hold = 7, morph = 1.6 } = {}) {
  if (!scenes.length || !Number.isFinite(hold) || !Number.isFinite(morph) || hold <= 0 || morph <= 0) {
    throw new Error('Provide at least one scene and positive finite hold/morph durations.');
  }
  const duration = hold + morph;
  return {
    scenes,
    duration,
    at(seconds, selected = null) {
      const time = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
      if (selected !== null) {
        const index = Math.max(0, Math.min(scenes.length - 1, Math.floor(Number.isFinite(selected) ? selected : 0)));
        return { from: scenes[index], to: scenes[index], age: time, nextAge: time, mix: 0 };
      }
      const index = Math.floor(time / duration) % scenes.length;
      const phase = time % duration;
      return {
        from: scenes[index], to: scenes[(index + 1) % scenes.length],
        age: phase + morph, nextAge: Math.max(0, phase - hold),
        mix: scenes.length === 1 ? 0 : smooth(Math.max(0, Math.min(1, (phase - hold) / morph))),
      };
    },
  };
}
