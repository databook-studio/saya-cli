# The mascot

saya is an owl: it watches everything and touches nothing, which is what
read-only means. Its pupils are terminal cursors — the one detail that keeps the
mark saya's rather than a generic bird, and the only thing the state system
changes.

## Terminal states

The pupils are two glyphs in one row of the block art. Substitute them and the
whole expression system falls out of it.

| State | Pupils | Behaviour |
| --- | --- | --- |
| idle | `▐` `▌` | steady, with an occasional blink |
| thinking | `▐` `▌` | blink quickly, or track side to side in SVG |
| answering | `█` `█` | solid and wide, no timer |
| error | `─` `─` | flat, rendered in red `#e5695f` |

Colours: body in iris `#9d8bf5`, shade in `#8171e6`, sockets and beak in ink
`#17151f`, pupils in the foreground. The pupils converge — `▐` in the left
socket, `▌` in the right — so the owl reads as focused rather than vacant.

## Assets

| File | Use |
| --- | --- |
| `saya-owl.svg` | the README header — ground shadow, blinking eyelid |
| `saya-owl-static.svg` | same, no animation (print, or anywhere motion is unwanted) |
| `saya-mark.svg` | the bare mark, no ground — favicons, avatars, small sizes |
| `states/*.svg` | the four states above |
| `splash.txt`, `splash-compact.txt` | the block art, mirrored in `ui/splash.rs` |

All the SVGs share one body path and one face geometry; edit them together or
the family drifts. Three rules the current shapes encode, each of which was a
mistake first:

- The shade is a **crescent** — the body filled in shade, then a large offset
  circle of the body colour clipped to the silhouette. A straight-edged band
  reads as a crease across a round form, and a half-and-half split bisects the
  face.
- Idle blinks with an **eyelid**, not by hiding the pupils. Fading a pupil out
  leaves a black void where the cursor was, which reads as an empty stare. The
  block art gets this free: its sockets are gaps, so a missing pupil is a closed
  eye.
- The pupils are **taller than they are wide**. Give them the sockets'
  proportions and they stop reading as cursors.

The block art is duplicated in `ui/splash.rs` rather than loaded from these
files, so the binary needs no assets at runtime. Change one, change both.
