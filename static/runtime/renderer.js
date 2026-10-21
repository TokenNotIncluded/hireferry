/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  hash,
  mix as lerp,
  smooth,

} from './geometry.js'

const pack = (r        , g        , b        ) =>
  ((255 << 24) |
    (Math.round(b) << 16) |
    (Math.round(g) << 8) |
    Math.round(r)) >>>
  0
/** Coverage and trails blend with the actual ground, never a black matte. */

export function posterPalette(background      ) {
  const light = background[0] + background[1] + background[2] > 384
  const coverage = (pixel        , alpha        ) =>
    pack(
      lerp(background[0], pixel & 255, alpha),
      lerp(background[1], (pixel >>> 8) & 255, alpha),
      lerp(background[2], (pixel >>> 16) & 255, alpha)
    )
  return {
    light,
    background: pack(...background),
    coverage,
    ink: (color      ) => {
      // Preserve hue and shading, but give pale materials a printable midtone.
      const luminance =
        color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722
      const tone = light ? 0.62 : 1
      const rgb = color.map((channel) => {
        const saturated = light
          ? luminance + (channel - luminance) * 1.2
          : channel
        return Math.min(255, Math.max(0, saturated * tone))
      })
      return coverage(pack(...rgb), light ? 1 : 0.96)
    },
  }
}

export function createRenderer(
  canvas                   ,
  timeline,
  onReady = () => {}
) {
  const context = canvas.getContext('2d', { alpha: false })
  if (!context) return null
  let alive = true,
    ready = false
  // Keep the fine-point lattice, sample budgets and brush springs.
  const count = canvas.clientWidth < 680 ? 22000 : 60000
  const offsets = new Float32Array(count * 4)
  const scatter = Float32Array.from(
    { length: count * 2 },
    (_, i) => hash(i) - 0.5
  )
  const stats = { frames: 0, points: 0, maxOffset: 0, time: 0, scene: "" }
  const position       = [0, 0, 0]
  let bitmap                       ,
    pixels                         ,
    depths
  let width = 0,
    height = 0,
    ratio = 1,
    lastFringe = -1
  let colors = new WeakMap                ()
  let palette = posterPalette([7, 7, 7])
  let themeKey
  const refreshTheme = () => {
    const key = document.documentElement.className
    if (key === themeKey) return
    themeKey = key
    const value = getComputedStyle(canvas)
      .getPropertyValue('--poster-ground')
      .trim()
    const hex = /^#([a-f\d]{6})$/i.exec(value)
    const color = hex ? Number.parseInt(hex[1], 16) : 0x070707
    palette = posterPalette([
      (color >>> 16) & 255,
      (color >>> 8) & 255,
      color & 255,
    ])
    colors = new WeakMap()
  }
  const paintFor = (points         , fringe        ) => {
    let paint = colors.get(points)
    if (!paint) {
      paint = {
        solid: new Uint32Array(points.length),
        edge: new Uint32Array(points.length),
        corner: new Uint32Array(points.length),
        trail: new Uint32Array(points.length),
      }
      for (let i = 0; i < points.length; i++) {
        const pixel = palette.ink(points[i].color)
        paint.solid[i] = pixel
        paint.edge[i] = palette.coverage(pixel, fringe)
        paint.corner[i] = palette.coverage(pixel, fringe * fringe)
        paint.trail[i] = palette.coverage(pixel, 0.12)
      }
      colors.set(points, paint)
    }
    return paint
  }
  return {
    draw(time        , pointer         , delta        , still = false, selected = null) {
      if (!alive) return
      const w = canvas.clientWidth,
        h = canvas.clientHeight
      if (!w || !h) return
      const nextRatio = Math.min(window.devicePixelRatio || 1, 1.5)
      if (width !== w || height !== h || ratio !== nextRatio) {
        ratio = nextRatio
        width = w
        height = h
        canvas.width = Math.round(w * ratio)
        canvas.height = Math.round(h * ratio)
        bitmap = context.createImageData(canvas.width, canvas.height)
        pixels = new Uint32Array(bitmap.data.buffer)
        depths = new Float32Array(canvas.width * canvas.height)
        themeKey = undefined
      }
      refreshTheme()
      const seconds = still ? 0 : Math.min(0.08, Math.max(0, Number.isFinite(delta) ? delta : 0))
      const left = timeline.at(time, selected)
      canvas.dataset.sculpture = left.mix < 0.5 ? left.from.id : left.to.id
      const grid = w < 680 ? 1.7 : 2.7,
        diameter = grid * (palette.light ? 0.96 : 0.83) * ratio
      const fullSize = Math.floor(diameter)
      const fringe = diameter - fullSize > 0.1 ? diameter - fullSize : 0
      const size = Math.max(1, fullSize + (fringe ? 1 : 0))
      if (fringe !== lastFringe) {
        colors = new WeakMap()
        lastFringe = fringe
      }
      const inputs = [
        { model: left.from, weight: 1-left.mix, age: left.age },
        { model: left.to, weight: left.mix, age: left.nextAge },
      ]
      const layers = inputs.filter(l => l.weight > 0).map(layer => {
        const model = layer.model, [yaw, pitch] = model.view ?? [0,0]
        return { ...layer, pose: model.animate(layer.age), paint: paintFor(model.points, fringe),
          cy: Math.cos(yaw), sy: Math.sin(yaw), cp: Math.cos(pitch), sp: Math.sin(pitch) }
      })
      const spread = Math.sin(Math.PI * left.mix) ** 2
      const n = Math.min(
        count,
        Math.max(...layers.map((l) => l.model.points.length))
      )
      if (!n || !bitmap || !pixels || !depths) return
      pixels.fill(palette.background)
      depths.fill(-Infinity)
      // Leave room for dispersal in every direction, then return to the resting frame.
      const scale = Math.min(w * 0.34, h * 0.35) * (1 - spread * 0.3),
        centerX = w / 2,
        centerY = h * (0.5 + spread * 0.03)
      const stride = canvas.width,
        rows = canvas.height
      const brushX = pointer.x - pointer.previousX,
        brushY = pointer.y - pointer.previousY
      const segment = brushX * brushX + brushY * brushY
      const speed = Math.min(1600, Math.hypot(pointer.vx, pointer.vy))
      const radius = Math.min(w, h) * (0.22 + speed / 22000)
      const recovery = Math.exp(-20 * seconds)
      let maxOffset = 0
      for (let i = 0; i < n; i++) {
        let x = 0,
          y = 0,
          z = 0,
          red = 0,
          green = 0,
          blue = 0
        let pixel = 0,
          edgePixel = 0,
          cornerPixel = 0,
          trail = 0
        for (const layer of layers) {
          const sample = Math.floor((i / n) * layer.model.points.length),
            p = layer.model.points[sample]
          position[0] = p.x
          position[1] = p.y
          position[2] = p.z
          layer.pose(p, position)
          const rx = position[0] * layer.cy + position[2] * layer.sy
          const rz = position[2] * layer.cy - position[0] * layer.sy
          x += rx * layer.weight
          y += (position[1] * layer.cp + rz * layer.sp) * layer.weight
          z += (rz * layer.cp - position[1] * layer.sp) * layer.weight
          pixel = layer.paint.solid[sample]
          if (layers.length === 1) {
            edgePixel = layer.paint.edge[sample]
            cornerPixel = layer.paint.corner[sample]
            trail = layer.paint.trail[sample]
          } else {
            red += (pixel & 255) * layer.weight
            green += ((pixel >>> 8) & 255) * layer.weight
            blue += ((pixel >>> 16) & 255) * layer.weight
          }
        }
        if (layers.length > 1) {
          pixel = pack(red, green, blue)
          edgePixel = palette.coverage(pixel, fringe)
          cornerPixel = palette.coverage(pixel, fringe * fringe)
          trail = palette.coverage(pixel, 0.12)
        }
        x += scatter[i * 2] * spread * 1.8
        y += (scatter[i * 2 + 1] * 1.25 + 0.16) * spread
        z += scatter[i * 2] * spread * 0.45
        const perspective = 3.9 / (3.9 - z * 0.42)
        const px = Math.round((centerX + x * scale * perspective) / grid) * grid
        const py = Math.round((centerY - y * scale * perspective) / grid) * grid
        const index = i * 4
        let fx = 0,
          fy = 0
        if (pointer.active && !still) {
          const along = segment
            ? Math.max(
                0,
                Math.min(
                  1,
                  ((px - pointer.previousX) * brushX +
                    (py - pointer.previousY) * brushY) /
                    segment
                )
              )
            : 1
          const dx = px - (pointer.previousX + brushX * along),
            dy = py - (pointer.previousY + brushY * along)
          const distance = Math.hypot(dx, dy)
          if (distance < radius) {
            const force = Math.pow(1 - distance / radius, 2)
            fx =
              force *
              (pointer.vx * 0.16 +
                dx * 0.25 +
                scatter[i * 2] * (35 + speed * 0.06))
            fy =
              force *
              (pointer.vy * 0.07 +
                dy * 0.35 +
                scatter[i * 2 + 1] * (30 + speed * 0.09))
          }
        }
        // Closed-form critical damping: return time does not depend on fps.
        const gapX = offsets[index] - fx,
          gapY = offsets[index + 1] - fy
        const springX = offsets[index + 2] + 20 * gapX,
          springY = offsets[index + 3] + 20 * gapY
        offsets[index] = fx + (gapX + springX * seconds) * recovery
        offsets[index + 1] = fy + (gapY + springY * seconds) * recovery
        offsets[index + 2] =
          (offsets[index + 2] - 20 * springX * seconds) * recovery
        offsets[index + 3] =
          (offsets[index + 3] - 20 * springY * seconds) * recovery
        const ox = offsets[index],
          oy = offsets[index + 1]
        maxOffset = Math.max(maxOffset, Math.hypot(ox, oy))
        const dx = Math.round((px + ox) * ratio),
          dy = Math.round((py + oy) * ratio)
        for (
          let row = Math.max(0, dy);
          row < Math.min(rows, dy + size);
          row++
        ) {
          for (
            let col = Math.max(0, dx);
            col < Math.min(stride, dx + size);
            col++
          ) {
            const slot = row * stride + col
            if (z < depths[slot]) continue
            depths[slot] = z
            pixels[slot] = fringe
              ? row - dy === fullSize && col - dx === fullSize
                ? cornerPixel
                : row - dy === fullSize || col - dx === fullSize
                  ? edgePixel
                  : pixel
              : pixel
          }
        }
        if (Math.abs(ox) > 2) {
          const ty = Math.round((py + oy * 0.75) * ratio),
            tx = Math.round((px + ox * 0.62) * ratio)
          const length = Math.round(
            (Math.min(55, Math.abs(ox) * 0.36) + grid) * ratio
          )
          if (ty >= 0 && ty < rows) {
            for (
              let col = Math.max(0, tx);
              col < Math.min(stride, tx + length);
              col++
            ) {
              const slot = ty * stride + col
              if (pixels[slot] === palette.background) pixels[slot] = trail
            }
          }
        }
      }
      context.putImageData(bitmap, 0, 0)
      Object.assign(stats, { frames: stats.frames+1, points: n, maxOffset, time, scene: canvas.dataset.sculpture })
      // Notify only after a real frame. A microtask avoids re-entering the owner.
      if (!ready) {
        ready = true
        canvas.dataset.ready = 'true'
        queueMicrotask(() => {
          if (alive) onReady()
        })
      }
    },
    inspect() { return { ...stats, ready } },
    dispose() {
      alive = false
      delete canvas.dataset.ready
      delete canvas.dataset.sculpture
      canvas.width = 0
      canvas.height = 0
      bitmap = undefined
      pixels = undefined
      depths = undefined
      colors = new WeakMap()
    },
  }
}
