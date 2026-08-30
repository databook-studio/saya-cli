# Terminal states

The art is one string. Only the cursor glyph changes — substitute the character
at the mouth position and the whole expression system falls out of it.

| State | Glyph | Behaviour |
| --- | --- | --- |
| idle | `▌` / ` ` | alternate on a ~575ms timer |
| thinking | `▌` / ` ` | alternate on a ~250ms timer |
| answering | `█` | solid, no timer |
| error | `─` | solid, rendered in red `#e5695f` |

Colours: body in iris `#9d8bf5`, cast-shadow row in a dimmed iris `#574f6e`.

## SVG assets

| File | Use |
| --- | --- |
| `saya-shadow.svg` | the README header — cast shadow, blinking cursor |
| `saya-shadow-static.svg` | same, no animation (print, or anywhere motion is unwanted) |
| `saya-mark.svg` | the bare mark, no shadow — favicons, avatars, small sizes |
| `states/*.svg` | the four terminal states above, as SVG |

All seven share one body path and one face geometry; edit them together or the
family drifts. Two rules the current shapes encode:

- The shade is a **rim** clipped to the lower-right edge (`M52 200 L200 52 L200
  200 Z`), not a half-and-half split. A vertical split bisects the face and
  reads as a crease rather than as light.
- The cursor is **narrower and taller** than the eyes (9×22 against 14×19). Give
  it the eyes' proportions and it reads as a third eye instead of a cursor.

The cursor blinks 72% on / 28% off — longer than a real terminal cursor, so a
screenshot or social-card thumbnail is unlikely to catch the mascot mouthless.
