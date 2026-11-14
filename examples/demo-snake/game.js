/*
 * Neon Snake
 * ----------
 * The game rules live in pure functions (no DOM, no timers) so they can be
 * unit-tested with Node's built-in test runner. The renderer only touches the
 * DOM when mount() is called from the browser.
 */
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) {
    module.exports = api;
  } else {
    root.NeonSnake = api;
  }
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  // ---------------------------------------------------------------- constants

  const DIRECTIONS = {
    up: { x: 0, y: -1 },
    down: { x: 0, y: 1 },
    left: { x: -1, y: 0 },
    right: { x: 1, y: 0 },
  };

  const OPPOSITE = { up: 'down', down: 'up', left: 'right', right: 'left' };

  const BASE_INTERVAL = 140; // ms per step at score 0
  const MIN_INTERVAL = 55; // fastest the snake ever gets
  const SPEED_STEP = 0.6; // ms shaved off per point of score
  const POINTS_PER_ORB = 10;
  const START_LENGTH = 3;
  const MAX_QUEUED_TURNS = 2;

  // ------------------------------------------------------------- pure logic

  /** Milliseconds between steps for a given score: faster as the score climbs. */
  function speedForScore(score) {
    const s = Math.max(0, Number(score) || 0);
    return Math.max(MIN_INTERVAL, BASE_INTERVAL - s * SPEED_STEP);
  }

  /** Pick a random free cell for the next orb. Returns null when the board is full. */
  function spawnOrb(state, rng) {
    const random = rng || state.rng || Math.random;
    const occupied = new Set(state.snake.map((s) => s.x + ',' + s.y));
    const free = [];
    for (let y = 0; y < state.rows; y++) {
      for (let x = 0; x < state.cols; x++) {
        if (!occupied.has(x + ',' + y)) free.push({ x, y });
      }
    }
    if (free.length === 0) {
      state.won = true;
      return null;
    }
    const i = Math.min(free.length - 1, Math.floor(random() * free.length));
    return free[i];
  }

  /** Fresh game state. `snake[0]` is the head. */
  function createState(options) {
    const opts = options || {};
    const cols = opts.cols || 25;
    const rows = opts.rows || 25;
    const rng = opts.rng || Math.random;
    const midY = Math.floor(rows / 2);
    const headX = Math.max(START_LENGTH - 1, Math.floor(cols / 4));

    const snake = [];
    for (let i = 0; i < START_LENGTH; i++) {
      snake.push({ x: headX - i, y: midY });
    }

    const state = {
      cols,
      rows,
      snake,
      dir: 'right',
      pending: [],
      score: 0,
      eaten: 0,
      over: false,
      won: false,
      cause: null,
      orb: null,
      rng,
    };
    state.orb = spawnOrb(state, rng);
    return state;
  }

  /**
   * Queue a turn. Reversing into yourself and repeating the current heading are
   * both ignored. Returns true when the turn was accepted.
   */
  function queueDirection(state, dir) {
    if (state.over || !DIRECTIONS[dir]) return false;
    const last = state.pending.length
      ? state.pending[state.pending.length - 1]
      : state.dir;
    if (dir === last || dir === OPPOSITE[last]) return false;
    if (state.pending.length >= MAX_QUEUED_TURNS) return false;
    state.pending.push(dir);
    return true;
  }

  /** Advance the game one step. Mutates and returns `state`. */
  function stepState(state) {
    if (state.over) return state;

    if (state.pending.length) state.dir = state.pending.shift();

    const delta = DIRECTIONS[state.dir];
    const head = state.snake[0];
    const next = { x: head.x + delta.x, y: head.y + delta.y };

    // Wall.
    if (
      next.x < 0 ||
      next.y < 0 ||
      next.x >= state.cols ||
      next.y >= state.rows
    ) {
      state.over = true;
      state.cause = 'wall';
      return state;
    }

    const willGrow =
      state.orb !== null && next.x === state.orb.x && next.y === state.orb.y;

    // The tail cell frees up this step unless we are growing into it.
    const body = willGrow
      ? state.snake
      : state.snake.slice(0, state.snake.length - 1);
    if (body.some((seg) => seg.x === next.x && seg.y === next.y)) {
      state.over = true;
      state.cause = 'self';
      return state;
    }

    state.snake.unshift(next);

    if (willGrow) {
      state.score += POINTS_PER_ORB;
      state.eaten += 1;
      state.orb = spawnOrb(state, state.rng);
    } else {
      state.snake.pop();
    }

    return state;
  }

  // ---------------------------------------------------------------- renderer

  const PALETTE = {
    bg: '#05010f',
    grid: 'rgba(0, 255, 255, 0.055)',
    head: '#8bfff9',
    bodyA: '#39f6ff',
    bodyB: '#b14bff',
    orb: '#ff2fd0',
    orbCore: '#fff0ff',
    text: '#e9f7ff',
    dim: 'rgba(233, 247, 255, 0.55)',
  };

  function roundRect(ctx, x, y, w, h, r) {
    const radius = Math.min(r, w / 2, h / 2);
    ctx.beginPath();
    ctx.moveTo(x + radius, y);
    ctx.arcTo(x + w, y, x + w, y + h, radius);
    ctx.arcTo(x + w, y + h, x, y + h, radius);
    ctx.arcTo(x, y + h, x, y, radius);
    ctx.arcTo(x, y, x + w, y, radius);
    ctx.closePath();
  }

  function drawGrid(ctx, state, cell) {
    ctx.save();
    ctx.strokeStyle = PALETTE.grid;
    ctx.lineWidth = 1;
    for (let x = 1; x < state.cols; x++) {
      ctx.beginPath();
      ctx.moveTo(x * cell + 0.5, 0);
      ctx.lineTo(x * cell + 0.5, state.rows * cell);
      ctx.stroke();
    }
    for (let y = 1; y < state.rows; y++) {
      ctx.beginPath();
      ctx.moveTo(0, y * cell + 0.5);
      ctx.lineTo(state.cols * cell, y * cell + 0.5);
      ctx.stroke();
    }
    ctx.restore();
  }

  function drawOrb(ctx, state, cell, time) {
    if (!state.orb) return;
    const cx = state.orb.x * cell + cell / 2;
    const cy = state.orb.y * cell + cell / 2;
    const pulse = 0.5 + 0.5 * Math.sin(time / 260);
    const r = cell * (0.26 + 0.06 * pulse);

    ctx.save();
    ctx.shadowColor = PALETTE.orb;
    ctx.shadowBlur = 18 + 14 * pulse;
    const grad = ctx.createRadialGradient(cx, cy, 0, cx, cy, r * 2.4);
    grad.addColorStop(0, PALETTE.orbCore);
    grad.addColorStop(0.35, PALETTE.orb);
    grad.addColorStop(1, 'rgba(255, 47, 208, 0)');
    ctx.fillStyle = grad;
    ctx.beginPath();
    ctx.arc(cx, cy, r * 2.4, 0, Math.PI * 2);
    ctx.fill();

    ctx.fillStyle = PALETTE.orbCore;
    ctx.beginPath();
    ctx.arc(cx, cy, r * 0.55, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  function drawSnake(ctx, state, cell, time) {
    const n = state.snake.length;
    ctx.save();
    ctx.shadowColor = PALETTE.bodyA;
    ctx.shadowBlur = 16;

    for (let i = n - 1; i >= 0; i--) {
      const seg = state.snake[i];
      const t = n === 1 ? 0 : i / (n - 1);
      const pad = cell * (0.09 + 0.05 * t);
      const x = seg.x * cell + pad;
      const y = seg.y * cell + pad;
      const size = cell - pad * 2;

      ctx.fillStyle = i === 0 ? PALETTE.head : mix(PALETTE.bodyA, PALETTE.bodyB, t);
      ctx.shadowColor = i === 0 ? PALETTE.head : PALETTE.bodyA;
      ctx.shadowBlur = i === 0 ? 26 : 14;
      roundRect(ctx, x, y, size, size, cell * 0.32);
      ctx.fill();
    }

    // Eyes on the head, looking where we are going.
    const head = state.snake[0];
    const d = DIRECTIONS[state.dir];
    const hx = head.x * cell + cell / 2;
    const hy = head.y * cell + cell / 2;
    const off = cell * 0.17;
    const fwd = cell * 0.16;
    const eyeR = Math.max(1.4, cell * 0.075);
    ctx.shadowBlur = 0;
    ctx.fillStyle = '#04121a';
    for (const side of [-1, 1]) {
      const ex = hx + d.x * fwd + (d.x === 0 ? side * off : 0);
      const ey = hy + d.y * fwd + (d.y === 0 ? side * off : 0);
      ctx.beginPath();
      ctx.arc(ex, ey, eyeR, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.restore();
  }

  function mix(a, b, t) {
    const pa = hexToRgb(a);
    const pb = hexToRgb(b);
    const r = Math.round(pa.r + (pb.r - pa.r) * t);
    const g = Math.round(pa.g + (pb.g - pa.g) * t);
    const bl = Math.round(pa.b + (pb.b - pa.b) * t);
    return 'rgb(' + r + ',' + g + ',' + bl + ')';
  }

  function hexToRgb(hex) {
    const h = hex.replace('#', '');
    return {
      r: parseInt(h.slice(0, 2), 16),
      g: parseInt(h.slice(2, 4), 16),
      b: parseInt(h.slice(4, 6), 16),
    };
  }

  function drawOverlay(ctx, state, w, h, time) {
    if (!state.over) return;
    ctx.save();
    ctx.fillStyle = 'rgba(5, 1, 15, 0.78)';
    ctx.fillRect(0, 0, w, h);

    const pulse = 0.6 + 0.4 * Math.sin(time / 320);
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';

    ctx.shadowColor = '#ff2fd0';
    ctx.shadowBlur = 28 * pulse;
    ctx.fillStyle = '#ff5ce0';
    ctx.font = '700 52px "Trebuchet MS", "Segoe UI", system-ui, sans-serif';
    ctx.fillText('GAME OVER', w / 2, h / 2 - 46);

    ctx.shadowBlur = 0;
    ctx.fillStyle = PALETTE.text;
    ctx.font = '600 22px "Trebuchet MS", "Segoe UI", system-ui, sans-serif';
    const why = state.cause === 'wall' ? 'You hit the wall.' : 'You bit yourself.';
    ctx.fillText(why, w / 2, h / 2 + 6);

    ctx.fillStyle = PALETTE.dim;
    ctx.font = '600 18px "Trebuchet MS", "Segoe UI", system-ui, sans-serif';
    ctx.fillText('Score ' + state.score, w / 2, h / 2 + 42);

    ctx.shadowColor = '#39f6ff';
    ctx.shadowBlur = 18 * pulse;
    ctx.fillStyle = '#8bfff9';
    ctx.font = '700 20px "Trebuchet MS", "Segoe UI", system-ui, sans-serif';
    ctx.fillText('Press SPACE to restart', w / 2, h / 2 + 92);
    ctx.restore();
  }

  function draw(ctx, state, cell, time) {
    const w = state.cols * cell;
    const h = state.rows * cell;
    ctx.save();
    ctx.fillStyle = PALETTE.bg;
    ctx.fillRect(0, 0, w, h);
    ctx.restore();

    drawGrid(ctx, state, cell);
    drawOrb(ctx, state, cell, time);
    drawSnake(ctx, state, cell, time);
    drawOverlay(ctx, state, w, h, time);
  }

  // ------------------------------------------------------------------ mount

  const KEY_TO_DIR = {
    ArrowUp: 'up',
    ArrowDown: 'down',
    ArrowLeft: 'left',
    ArrowRight: 'right',
    w: 'up',
    a: 'left',
    s: 'down',
    d: 'right',
    W: 'up',
    A: 'left',
    S: 'down',
    D: 'right',
  };

  const RESTART_KEYS = new Set([' ', 'Spacebar', 'Enter', 'r', 'R']);

  /**
   * Wire the game to a canvas. Returns a small handle so the page (or a
   * console) can inspect and drive it.
   */
  function mount(canvas, options) {
    const opts = options || {};
    const cell = opts.cell || 24;
    const cols = opts.cols || 25;
    const rows = opts.rows || 25;
    const ctx = canvas.getContext('2d');
    canvas.width = cols * cell;
    canvas.height = rows * cell;

    const scoreEl = opts.scoreEl || null;
    const speedEl = opts.speedEl || null;

    let game = createState({ cols, rows });
    let acc = 0;
    let last = 0;
    let raf = null;

    function syncHud() {
      if (scoreEl) scoreEl.textContent = String(game.score);
      if (speedEl) {
        const interval = speedForScore(game.score);
        const level = Math.round(((BASE_INTERVAL - interval) / (BASE_INTERVAL - MIN_INTERVAL)) * 100);
        speedEl.textContent = level + '%';
      }
    }

    function restart() {
      game = createState({ cols, rows });
      acc = 0;
      last = 0;
      syncHud();
    }

    function frame(ts) {
      if (!last) last = ts;
      const dt = Math.min(ts - last, 250);
      last = ts;

      if (!game.over) {
        acc += dt;
        let interval = speedForScore(game.score);
        while (acc >= interval && !game.over) {
          acc -= interval;
          stepState(game);
          interval = speedForScore(game.score);
        }
        syncHud();
      }

      draw(ctx, game, cell, ts);
      raf = requestAnimationFrame(frame);
    }

    function onKeyDown(event) {
      const dir = KEY_TO_DIR[event.key];
      if (dir) {
        event.preventDefault();
        queueDirection(game, dir);
        return;
      }
      if (RESTART_KEYS.has(event.key)) {
        event.preventDefault();
        if (game.over) restart();
      }
    }

    window.addEventListener('keydown', onKeyDown);
    syncHud();
    raf = requestAnimationFrame(frame);

    return {
      get state() {
        return game;
      },
      restart,
      stop() {
        if (raf !== null) cancelAnimationFrame(raf);
        raf = null;
        window.removeEventListener('keydown', onKeyDown);
      },
    };
  }

  return {
    DIRECTIONS,
    OPPOSITE,
    BASE_INTERVAL,
    MIN_INTERVAL,
    POINTS_PER_ORB,
    START_LENGTH,
    createState,
    queueDirection,
    stepState,
    spawnOrb,
    speedForScore,
    draw,
    mount,
  };
});
