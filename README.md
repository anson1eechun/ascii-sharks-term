# sharks

**English** | [繁體中文](README.zh-TW.md)

ASCII sharks hunting schools of fish in your terminal, in 24-bit colour. A small (~400 KB), dependency-free Rust port of [stackedlol/ascii-sharks](https://github.com/stackedlol/ascii-sharks), with mouse interaction.

<p align="center">
  <img src="docs/screenshot.svg" alt="sharks in a terminal: a great hammerhead, a tiger shark, schools of fish and a ripple" width="100%">
</p>

<sub>A real frame from the renderer, exported to SVG: a great hammerhead (right), a tiger shark (bottom) and a click ripple (left).</sub>

## Features

- **Eight shark species** with their own outlines, fins and markings: hammerheads, tiger, mako, thresher, nurse, zebra and a classic requiem shark.
- **Living fish schools.** Hundreds of fish school as boids. Alarm spreads through a school, and fleeing fish break sideways around a shark's path. Sharks pick off stragglers, and eaten fish come back later as new schools.
- **Mouse interaction.** Move the mouse to scare the fish. Click to make a ripple: the fish scatter, and nearby sharks turn tail and stop hunting for a few seconds.
- **Light on resources.** About 2 MB of memory, and 6–8% of one CPU core at 30 fps (3% at 10 fps).
- Screensaver mode, pause, live resizing, English and Chinese messages.

## Install

### Homebrew (macOS, Linux)

```sh
brew install anson1eechun/tap/sharks
```

### npm

```sh
npm install -g ascii-sharks-term
# or run it without installing:
npx ascii-sharks-term
```

The package ships prebuilt native binaries. A tiny shell launcher `exec`s the right one, so no Node.js process keeps running behind it.

### Cargo

```sh
cargo install --locked --git https://github.com/anson1eechun/ascii-sharks-term sharks
```

### Prebuilt binaries

Download a `.tar.gz` for your platform from [Releases](https://github.com/anson1eechun/ascii-sharks-term/releases) and put `sharks` on your `PATH`. On macOS, a binary downloaded through a browser is quarantined, so clear that once with `xattr -d com.apple.quarantine sharks`.

**Platforms:** macOS (Apple Silicon and Intel) and Linux (x86_64 and arm64; static musl builds). On Windows, use WSL.

## Usage

```sh
sharks                             # four classic sharks
sharks -k greathammer,tiger,mako   # pick the sharks; the first is biggest
sharks -S                          # screensaver: any key quits
sharks -f 20                       # fewer frames per second, less CPU
```

| Option | |
|---|---|
| `-k, --sharks <kind,...>` | Which sharks to show. The first is biggest and the rest step down. |
| `-l, --list` | List the shark kinds. |
| `-f, --fps <n>` | Frames per second, 1–120 (default 30). |
| `-S, --screensaver` | Any key quits. |
| `--aspect <n>` | Cell height ÷ width, if the auto-detected one looks off. |
| `-V, --version` / `-h, --help` | |

| While running | |
|---|---|
| `q`, `Esc`, `Ctrl-C` | Quit. |
| `p` | Pause. |
| Mouse move | Fish flee the pointer. |
| Click | Ripple: fish scatter, nearby sharks flee. |

You need a terminal with 24-bit colour and mouse reporting, such as Ghostty, iTerm2, WezTerm, kitty or Alacritty. Fullscreen looks best.

### Shark kinds

| `-k` name | Shark |
|---|---|
| `classic` | Classic requiem shark, cool blue-grey |
| `hammerhead` | Smooth hammerhead: swept-back head, bronze-olive |
| `greathammer` | Great hammerhead: wide, straight, notched head and a towering dorsal |
| `tiger` | Tiger shark: blunt snout, barred back |
| `mako` | Shortfin mako: conical snout, torpedo body, metallic indigo |
| `thresher` | Thresher shark: a whip of a tail as long as its body |
| `nurse` | Nurse shark: broad head, paddle fins, tawny |
| `zebra` | Zebra shark: spotted, with a long ribbon tail |

### Language

Messages follow your locale (`LC_ALL`, `LC_MESSAGES`, `LANG`). To choose explicitly, set `SHARKS_LANG=en` or `SHARKS_LANG=zh-TW`.

## How faithful is it?

The simulation (`world.ts`) and the renderer (`renderer.ts`) are ported statement for statement. The port keeps `f32` wherever upstream uses `Float32Array`, and the same random generator can drive both versions. [`verify/verify.sh`](verify/verify.sh) runs upstream's TypeScript and the Rust port on the same seed and compares 34 frames, cell by cell (glyph, foreground, background). Half of those frames force eating and respawning. All 34 match exactly.

After about 2 seconds the two runs drift apart. This is expected: the maths libraries differ in the last bit (about 1e-16), and the chaotic fish simulation amplifies it.

Differences from upstream:

- **No grid lines.** A terminal can't draw between cells. Upstream's disabled ambient dots are also left out.
- **Speed-ups that don't change the result.** Shark rasterisation skips spine segments and cells that provably can't be hit. Unit tests guard the bounds these rely on.
- **New: click ripples and sharks fleeing them.** Upstream's splash is unchanged. Without clicks, the new code never runs, so the comparison above still holds.

## Performance

Measured on Apple Silicon with a 200×55 terminal, 4 sharks and 340 fish:

| | CPU (one core) | Memory |
|---|---|---|
| `sharks -f 10` | 3.1% | 2.3 MB |
| `sharks -f 20` | 6.2% | 2.3 MB |
| `sharks` (30 fps) | 6–8% | 2.3 MB |
| `sharks -f 60` | 12.7% | 2.3 MB |
| for comparison: cmatrix | 4% | 2 MB |
| for comparison: asciiquarium | 5% | 11 MB |

Most of the cost is upstream's simulation (fish schooling) and its anti-aliased shark rendering (five samples per cell).

## Development

```sh
git clone --recurse-submodules https://github.com/anson1eechun/ascii-sharks-term
cd ascii-sharks-term/rs
cargo test --release           # unit tests
cargo run --release            # try it
../verify/verify.sh            # compare against upstream (needs Bun)
```

| Path | |
|---|---|
| `rs/src/world.rs`, `renderer.rs`, `math.rs`, `species.rs` | Ports of the matching upstream `.ts` files |
| `rs/src/term.rs` | Terminal I/O on plain libc: raw mode, size, signals, input, a diffing 24-bit ANSI writer |
| `rs/src/main.rs` | Options, frame loop, keyboard and mouse (replaces upstream's `engine.ts`) |
| `upstream/` | Git submodule of the original, pinned to the verified commit |
| `cell-canvas.ts`, `sharks.ts` | An earlier Bun port. `verify/` uses it to run upstream in a terminal-sized grid |
| `npm/` | npm package: the launcher and its manifest |

**Releasing.** Bump the version in `rs/Cargo.toml`, then push a tag:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

GitHub Actions builds macOS and Linux binaries and publishes a release. Then run `scripts/publish.sh v0.2.0` to publish the npm package and update the Homebrew formula.

**Upstream updates.** Bump the submodule, port the changes in `upstream/src` to `rs/src`, and run `verify/verify.sh`. It must report 0 mismatched.

## Credits

The simulation, the shark designs and the look are [ascii-sharks](https://github.com/stackedlol/ascii-sharks) by drewdi, released under the MIT License. This project ports them to the terminal.

## License

[MIT](LICENSE): free to use, modify and distribute, including commercially. Keep the copyright notices, including the original author's.
