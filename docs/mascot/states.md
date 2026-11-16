# The mascot

saya is a kangaroo: upright, alert, always ready to leap. The state lives in the
one visible eye — the pouch is a quiet lip and holds nothing.

## Concept

A kangaroo stands in three-quarter side view, facing right. It has a small head
with a long muzzle and the nose at the tip; two tall, leaf-shaped ears with
rounded tips; a pear-shaped body leaning slightly forward, with a narrow chest
and wide hips; a large rounded haunch at the back of the hips; two short
forearms held in front of the chest; one long hind foot flat on the ground,
pointing forward; and a thick tail that tapers to a point on the ground behind
it, forming a tripod with the foot. Those shapes — the long ears, the muzzle,
the haunch, the long foot and the tapered tail — carry the species; everything
else is face.

The face reports the state. Idle has a round eye. Thinking half-closes the eye
and blinks. Answering opens it wide with a highlight. Error flattens it to a red
line. The pouch is a lip across the belly, nothing more: no cursor, no glyph, no
second thing competing with the eye.

## Palette

| Token | Hex | Use |
| --- | --- | --- |
| iris | `#9d8bf5` | body, head, tail, eye highlights; ground shadow at 20% |
| shade | `#8171e6` | haunch, inner ears, hind foot, forearms, pouch lip |
| ink | `#17151f` | eye, nose, mouth |
| error | `#e5695f` | the error eye only |

The body is filled in shade, then a large offset circle of iris is clipped to
the silhouette. What the circle misses is a crescent rim that follows the form;
the haunch is the large shade mass at the back of the hips. A straight-edged
band reads as a crease; a half-and-half split bisects the body.

## States

The eye is the state. The pouch does not change.

| State | Eye | Behaviour |
| --- | --- | --- |
| idle | round | steady; the hero blinks gently every 5.4s |
| thinking | half-closed | the lid sinks and lifts, slowly |
| answering | wide open, with highlight | held wide, no animation |
| error | flat line in `#e5695f` | static, red |

The hero (`saya-kangaroo.svg`) also gives each ear a small twitch. The static
file and the idle/answering/error states do not move. Thinking's lid is the one
state whose motion is part of the state.

## Assets

| File | Use |
| --- | --- |
| `saya-kangaroo.svg` | README header — ground shadow, gentle eye blink and ear twitch |
| `saya-kangaroo-static.svg` | same, no animation (print, or anywhere motion is unwanted) |
| `saya-mark.svg` | head in profile — snout, two tall ears, one eye — filling the viewBox; favicons, avatars, 16px |
| `states/*.svg` | the four states above |

All the SVGs share one body path, one face geometry, one tail and one hind foot;
edit them together or the family drifts. The rules the shapes encode, each of
which was a mistake first:

- The figure is a **three-quarter side view facing right**, not front-on. One
  eye is visible; the far eye is hidden by the muzzle. A front-on face reads as
  a cat or a fox.
- The head is **small relative to the body**, with a **long muzzle** and the
  nose at the tip. A round head with a centred nose loses the species.
- The ears are **long, upright leaves with rounded tips**, not short pointy
  triangles.
- The body is **pear-shaped and leans slightly forward**: narrow chest, wide
  hips. A large rounded **haunch** sits at the back of the hips. A straight
  barrel reads as a generic animal.
- The hind foot is **one long flat foot** pointing right, clearly longer than
  the head. Do not draw two separate slabs.
- The tail is **thick at the base, long and tapered to a point**, lying on the
  ground behind (left), forming a tripod with the foot. A short, bushy tail
  reads as a squirrel.
- The arms are **two short forearms with small paws**, held in front of the
  chest. They do not hang like a second pair of legs.
- The pouch is a **lip, not a hole**, and it sits on the belly only. A shade
  band sags across the belly; a filled ink pocket reads as a hole through the
  body at 16px; an empty lip reads as a pocket.
- The shade is a **crescent**. The body is filled in shade, then a large offset
  circle of the body colour is clipped to the silhouette. The rim follows the
  curve; the haunch is the shade mass at the rear. A straight band reads as a
  crease.
- The **eye is the only state channel**. Do not add a second glyph.
- The viewBox is **cropped to the figure plus its shadow**: the content is
  centred with even margins (about 28px left/right, 12px top/bottom) rather than
  sitting in a corner of an uncropped 200px square.
- The mark is the **head alone** — snout, two tall ears, one eye — and fills its
  viewBox so it still reads at 16×16.

## Terminal mascot

The TUI home screen paints a tiny block-glyph kangaroo — four rows by ten
columns, in profile facing right: two tall ears, a head with one eye gap and
a snout, a body, and a tail sweeping left to the ground. It is drawn in the
iris accent:

```text
    ██ ██
  ███ ███▐
 ███████
▄▄█████▀
```

It is one `const` in `ui/surface/splash.rs`, so the owner can redraw it. The
SVG kangaroo above stays the docs mascot; the terminal does not render it.
