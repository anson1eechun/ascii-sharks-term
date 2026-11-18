# Changelog

## 0.3.0 - 2026-11-19

### Added
- Clock mode: `--clock[=style]` in seven styles (`block`, `shadow`, `hash`, `half`, `dots`, `big`, `lcd`), `--seconds`, and `--styles` to preview them.
- `-m` puts a message on the water, under the clock if there is one.
- Feeding (right click or `f`), rain (`--rain`), and shark names on hover.
- `--seed`, `--fish`, `--speed`, `--calm`, `--still`, and `--print` for one frame.
- `--color` (truecolor, 256, 16, mono; `NO_COLOR` is respected) and `--ascii`.
- Detects terminals that draw `·` two columns wide, and the cell size, at startup.
- Ctrl-Z suspends; schools grow and shrink with the window.

### Fixed
- Slow frame rates and fast `--speed` no longer slow the simulation down.
- `LINES=0`, tiny windows, control characters in `-m`, and half-overwritten wide glyphs.
- Signals arriving just before a wait are no longer missed.

### Changed
- The binary is 14% smaller (opt-level "s") at the same frame cost.

## 0.2.0 - 2026-11-18

### Added
- A random lineup of shark kinds and sizes by default.
- Releases publish to npm and Homebrew automatically from a version tag.

## 0.1.0 - 2026-11-18

- First release: stackedlol/ascii-sharks in the terminal, with click ripples that sharks avoid.
