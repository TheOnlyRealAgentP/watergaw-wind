// Animated wind particles for a MapLibre (or Mapbox) map.
//
//   const anim = new WindParticles(map, (lat, lon) => wind.at(lat, lon));
//   anim.start();     // rebuilds automatically after the map moves
//   anim.destroy();
//
// Particles live in screen space and move through a vector grid sampled from
// your wind function whenever the map stops moving, leaving fading trails.

export const WIND_COLORS = [[0, '#6fa8dc'], [4, '#64d2ff'], [8, '#30d158'], [12, '#ffd60a'], [17, '#ff9f0a'], [24, '#ff453a'], [33, '#ff2dbe']];

export function windColor(ms, stops = WIND_COLORS) {
  let c = stops[0][1];
  for (const [v, col] of stops) if (ms >= v) c = col;
  return c;
}

export class WindParticles {
  /**
   * @param map    MapLibre map
   * @param vector (lat, lon) => [u, v] m/s, or null where there's no data
   * @param opts   { density: px² per particle, speed: px per frame per m/s, maxAge: frames, colors }
   */
  constructor(map, vector, opts = {}) {
    this.map = map;
    this.vector = vector;
    this.opts = { density: 260, speed: 0.22, maxAge: 90, colors: WIND_COLORS, max: 3500, ...opts };
    this.canvas = document.createElement('canvas');
    Object.assign(this.canvas.style, { position: 'absolute', inset: '0', pointerEvents: 'none', zIndex: 1 });
    map.getContainer().appendChild(this.canvas);
    this.ctx = this.canvas.getContext('2d');
    this.onStart = () => this.pause();
    this.onEnd = () => this.start();
    map.on('movestart', this.onStart);
    map.on('moveend', this.onEnd);
    this.ro = new ResizeObserver(() => this.running && this.start());
    this.ro.observe(map.getContainer());
  }

  setVector(vector) {
    this.vector = vector;
    this.start();
  }

  destroy() {
    this.pause();
    this.map.off('movestart', this.onStart);
    this.map.off('moveend', this.onEnd);
    this.ro.disconnect();
    this.canvas.remove();
  }

  pause() {
    this.running = false;
    cancelAnimationFrame(this.raf);
    this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
  }

  start() {
    const el = this.map.getContainer();
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    const W = el.clientWidth, H = el.clientHeight;
    if (!W || !H) return;
    this.canvas.width = W * dpr;
    this.canvas.height = H * dpr;
    this.canvas.style.width = `${W}px`;
    this.canvas.style.height = `${H}px`;
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const step = 24;
    const gx = Math.ceil(W / step) + 1, gy = Math.ceil(H / step) + 1;
    const data = new Float32Array(gx * gy * 3);
    for (let j = 0; j < gy; j++) for (let i = 0; i < gx; i++) {
      const ll = this.map.unproject([i * step, j * step]);
      const w = this.vector(ll.lat, ll.lng);
      const k = (j * gx + i) * 3;
      if (!w) { data[k + 2] = -1; continue; }
      data[k] = w[0];
      data[k + 1] = w[1];
      data[k + 2] = Math.hypot(w[0], w[1]);
    }
    this.grid = { gx, gy, step, data, W, H };
    const n = Math.round(Math.min(this.opts.max, (W * H) / this.opts.density));
    this.parts = Array.from({ length: n }, () => this.spawn({}));
    this.running = true;
    cancelAnimationFrame(this.raf);
    this.loop();
  }

  spawn(p) {
    p.x = Math.random() * this.grid.W;
    p.y = Math.random() * this.grid.H;
    p.age = Math.floor(Math.random() * this.opts.maxAge);
    return p;
  }

  sample(x, y) {
    const g = this.grid;
    const i = Math.round(x / g.step), j = Math.round(y / g.step);
    if (i < 0 || j < 0 || i >= g.gx || j >= g.gy) return null;
    const k = (j * g.gx + i) * 3;
    return g.data[k + 2] < 0 ? null : [g.data[k], g.data[k + 1], g.data[k + 2]];
  }

  loop() {
    if (!this.running) return;
    const ctx = this.ctx;
    const { W, H } = this.grid;
    ctx.globalCompositeOperation = 'destination-in';
    ctx.fillStyle = 'rgba(0,0,0,0.9)';
    ctx.fillRect(0, 0, W, H);
    ctx.globalCompositeOperation = 'source-over';
    ctx.lineWidth = 1.4;
    const k = this.opts.speed;
    const byColor = new Map();
    for (const p of this.parts) {
      const w = this.sample(p.x, p.y);
      if (!w || p.age++ > this.opts.maxAge) { this.spawn(p); p.age = 0; continue; }
      const nx = p.x + w[0] * k, ny = p.y - w[1] * k;
      const c = windColor(w[2], this.opts.colors);
      let list = byColor.get(c);
      if (!list) byColor.set(c, (list = []));
      list.push(p.x, p.y, nx, ny);
      p.x = nx;
      p.y = ny;
      if (nx < 0 || ny < 0 || nx > W || ny > H) this.spawn(p);
    }
    for (const [c, l] of byColor) {
      ctx.strokeStyle = c;
      ctx.beginPath();
      for (let i = 0; i < l.length; i += 4) { ctx.moveTo(l[i], l[i + 1]); ctx.lineTo(l[i + 2], l[i + 3]); }
      ctx.stroke();
    }
    this.raf = requestAnimationFrame(() => this.loop());
  }
}
