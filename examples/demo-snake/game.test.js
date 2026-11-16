'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');

const {
  BASE_INTERVAL,
  MIN_INTERVAL,
  POINTS_PER_ORB,
  START_LENGTH,
  createState,
  queueDirection,
  stepState,
  spawnOrb,
  speedForScore,
} = require('./game.js');

/** Deterministic PRNG so orb placement is reproducible. */
function mulberry32(seed) {
  let a = seed >>> 0;
  return function () {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Build a hand-made state for collision / edge-case tests. */
function makeState(overrides) {
  const base = {
    cols: 5,
    rows: 5,
    snake: [{ x: 2, y: 2 }],
    dir: 'right',
    pending: [],
    score: 0,
    eaten: 0,
    over: false,
    won: false,
    cause: null,
    orb: { x: 4, y: 4 },
    rng: mulberry32(1),
  };
  return Object.assign(base, overrides);
}

const onSnake = (state, cell) =>
  state.snake.some((s) => s.x === cell.x && s.y === cell.y);

// ---------------------------------------------------------------- new game

test('a new game starts with a snake, a heading, and no score', () => {
  const state = createState({ cols: 25, rows: 25, rng: mulberry32(7) });

  assert.equal(state.snake.length, START_LENGTH);
  assert.equal(state.dir, 'right');
  assert.equal(state.score, 0);
  assert.equal(state.over, false);
  assert.equal(state.cause, null);
  assert.deepEqual(state.pending, []);
});

test('the starting snake is contiguous and inside the board', () => {
  const state = createState({ cols: 25, rows: 25, rng: mulberry32(7) });

  for (const seg of state.snake) {
    assert.ok(seg.x >= 0 && seg.x < state.cols, 'x in bounds');
    assert.ok(seg.y >= 0 && seg.y < state.rows, 'y in bounds');
  }
  for (let i = 1; i < state.snake.length; i++) {
    const dx = Math.abs(state.snake[i].x - state.snake[i - 1].x);
    const dy = Math.abs(state.snake[i].y - state.snake[i - 1].y);
    assert.equal(dx + dy, 1, 'segments are adjacent');
  }
});

test('the first orb never spawns on top of the snake', () => {
  for (let seed = 0; seed < 50; seed++) {
    const state = createState({ cols: 12, rows: 12, rng: mulberry32(seed) });
    assert.ok(state.orb, 'an orb exists');
    assert.equal(onSnake(state, state.orb), false);
  }
});

// ------------------------------------------------------------------- speed

test('the snake speeds up as the score climbs', () => {
  assert.equal(speedForScore(0), BASE_INTERVAL);
  assert.ok(speedForScore(10) < speedForScore(0));
  assert.ok(speedForScore(50) < speedForScore(10));
});

test('speed is clamped so the game stays playable', () => {
  assert.equal(speedForScore(100000), MIN_INTERVAL);
  assert.ok(speedForScore(100000) >= MIN_INTERVAL);
});

test('a negative or junk score does not speed the snake up', () => {
  assert.equal(speedForScore(-50), BASE_INTERVAL);
  assert.equal(speedForScore(undefined), BASE_INTERVAL);
});

// --------------------------------------------------------------- steering

test('arrow keys queue a turn', () => {
  const state = createState({ rng: mulberry32(3) });
  assert.equal(queueDirection(state, 'up'), true);
  assert.deepEqual(state.pending, ['up']);
});

test('a 180-degree turn is refused', () => {
  const state = createState({ rng: mulberry32(3) });
  assert.equal(queueDirection(state, 'left'), false);
  assert.deepEqual(state.pending, []);
});

test('repeating the current heading is refused', () => {
  const state = createState({ rng: mulberry32(3) });
  assert.equal(queueDirection(state, 'right'), false);
  assert.deepEqual(state.pending, []);
});

test('a reversal is judged against the last queued turn, not the heading', () => {
  const state = createState({ rng: mulberry32(3) });
  assert.equal(queueDirection(state, 'up'), true);
  assert.equal(queueDirection(state, 'down'), false, 'cannot undo the queued turn');
  assert.equal(queueDirection(state, 'left'), true, 'left is fine after up');
  assert.deepEqual(state.pending, ['up', 'left']);
});

test('queued turns are capped so fast key mashing cannot stack up', () => {
  const state = createState({ rng: mulberry32(3) });
  assert.equal(queueDirection(state, 'up'), true);
  assert.equal(queueDirection(state, 'left'), true);
  assert.equal(queueDirection(state, 'down'), false);
  assert.equal(state.pending.length, 2);
});

test('steering is ignored once the game is over', () => {
  const state = makeState({ over: true, cause: 'wall' });
  assert.equal(queueDirection(state, 'up'), false);
  assert.deepEqual(state.pending, []);
});

// ---------------------------------------------------------------- movement

test('a step moves the head one cell and keeps the length', () => {
  const state = makeState();
  stepState(state);

  assert.deepEqual(state.snake[0], { x: 3, y: 2 });
  assert.equal(state.snake.length, 1);
  assert.equal(state.over, false);
});

test('a queued turn is applied on the next step', () => {
  const state = makeState({ snake: [{ x: 2, y: 2 }, { x: 1, y: 2 }] });
  queueDirection(state, 'up');
  stepState(state);

  assert.deepEqual(state.snake[0], { x: 2, y: 1 });
  assert.equal(state.dir, 'up');
  assert.deepEqual(state.pending, []);
});

test('a step after game over changes nothing', () => {
  const state = makeState({ over: true, cause: 'wall' });
  const before = JSON.stringify(state.snake);
  stepState(state);
  assert.equal(JSON.stringify(state.snake), before);
});

// ------------------------------------------------------------------ orbs

test('eating an orb grows the snake and adds score', () => {
  const state = makeState({
    snake: [{ x: 2, y: 2 }, { x: 1, y: 2 }],
    orb: { x: 3, y: 2 },
  });

  stepState(state);

  assert.equal(state.snake.length, 3, 'grew by one');
  assert.equal(state.score, POINTS_PER_ORB);
  assert.equal(state.eaten, 1);
  assert.deepEqual(state.snake[0], { x: 3, y: 2 });
});

test('eating an orb respawns it somewhere off the snake', () => {
  const state = makeState({
    snake: [{ x: 2, y: 2 }, { x: 1, y: 2 }],
    orb: { x: 3, y: 2 },
  });

  stepState(state);

  assert.ok(state.orb, 'a new orb exists');
  assert.equal(onSnake(state, state.orb), false);
});

test('the orb stays off the snake across a long simulated game', () => {
  const state = createState({ cols: 15, rows: 15, rng: mulberry32(99) });
  const turns = ['up', 'right', 'down', 'left'];

  for (let i = 0; i < 400 && !state.over; i++) {
    if (i % 3 === 0) queueDirection(state, turns[i % turns.length]);
    stepState(state);
    if (state.orb) assert.equal(onSnake(state, state.orb), false);
  }
});

test('spawnOrb reports a full board instead of looping forever', () => {
  const state = makeState({
    cols: 2,
    rows: 2,
    snake: [{ x: 0, y: 0 }, { x: 0, y: 1 }, { x: 1, y: 1 }, { x: 1, y: 0 }],
  });

  assert.equal(spawnOrb(state), null);
  assert.equal(state.won, true);
});

test('filling the last cell wins the game', () => {
  const state = makeState({
    cols: 2,
    rows: 2,
    snake: [{ x: 0, y: 0 }, { x: 0, y: 1 }, { x: 1, y: 1 }],
    dir: 'right',
    orb: { x: 1, y: 0 },
  });

  stepState(state);

  assert.equal(state.snake.length, 4);
  assert.equal(state.won, true);
  assert.equal(state.orb, null);
  assert.equal(state.over, false, 'winning is not a crash');
});

// ------------------------------------------------------------- game over

test('hitting a wall ends the game', () => {
  const state = makeState({ snake: [{ x: 0, y: 2 }], dir: 'left' });
  stepState(state);

  assert.equal(state.over, true);
  assert.equal(state.cause, 'wall');
});

test('hitting a wall on the top edge ends the game', () => {
  const state = makeState({ snake: [{ x: 2, y: 0 }], dir: 'up' });
  stepState(state);

  assert.equal(state.over, true);
  assert.equal(state.cause, 'wall');
});

test('running into your own body ends the game', () => {
  const state = makeState({
    snake: [
      { x: 2, y: 2 },
      { x: 2, y: 3 },
      { x: 1, y: 3 },
      { x: 1, y: 2 },
      { x: 1, y: 1 },
    ],
    dir: 'left',
  });

  stepState(state);

  assert.equal(state.over, true);
  assert.equal(state.cause, 'self');
});

test('following your own tail is legal, because the tail moves away', () => {
  const state = makeState({
    snake: [
      { x: 1, y: 2 },
      { x: 2, y: 2 },
      { x: 2, y: 3 },
      { x: 1, y: 3 },
    ],
    dir: 'down',
    orb: { x: 4, y: 4 },
  });

  stepState(state);

  assert.equal(state.over, false);
  assert.deepEqual(state.snake[0], { x: 1, y: 3 });
  assert.equal(state.snake.length, 4);
});

test('a crash leaves the score untouched', () => {
  const state = makeState({
    snake: [{ x: 0, y: 2 }],
    dir: 'left',
    score: 30,
  });

  stepState(state);

  assert.equal(state.over, true);
  assert.equal(state.score, 30);
});
