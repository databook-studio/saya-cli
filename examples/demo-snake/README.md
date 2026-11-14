# Neon Snake

A small, self-contained browser game built by saya from one prompt in the
[demo video](../../docs/media/demo-build-game.mp4). Open `index.html` in a
browser to play: arrow keys steer, orbs grow the snake and add score, the
snake speeds up as you score, and a wall or self-hit ends the run (press a
key to restart).

The game rules live in `game.js` as pure functions so they can be tested
without a DOM. Run the suite with Node's built-in runner:

```bash
node --test
```

The recording used `deepseek/deepseek-v4.1-flash-1017` through an
OpenAI-compatible endpoint, and the generated suite passes 25/25.
