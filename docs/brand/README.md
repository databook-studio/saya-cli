# Brand assets

The ring mark is the saya logo. The kangaroo is the terminal mascot, not the
logo; it lives in [`docs/mascot/`](../mascot/).

## Files

| File | Use |
| --- | --- |
| `saya-rings.svg` | the ring mark alone, drawn in `currentColor` — use it wherever the host page supplies the colour |
| `saya-logo-light.svg` | full lockup on a light background |
| `saya-logo-dark.svg` | full lockup on a dark background |
| `saya-logo-transparent.svg` | full lockup with a transparent background, for light surfaces |
| `saya-logo-transparent-dark.svg` | full lockup with a transparent background, for dark surfaces |
| `saya-lockup-horizontal.svg` | the mark and wordmark side by side |
| `saya-favicon.svg` | the ring mark cropped for a favicon/avatar |

The README header uses the transparent pair through `<picture>`, switching on
`prefers-color-scheme`.

## Colours

| Token | Hex | Use |
| --- | --- | --- |
| iris | `#9d8bf5` | the signature accent: rings, wordmark highlight |
| lavender | `#b9a6da` → `#a48ec8` | the gradient through the ring set |
| ink | `#0e0d14` | dark surfaces and the wordmark on light backgrounds |
