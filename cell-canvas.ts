/**
 * Stands in for the browser canvas so upstream's AsciiRenderer runs unmodified.
 * The renderer only ever draws cell-aligned: a large fillRect is a shark's skin
 * (cell background), a small fillRect is a wake dot, fillText is one glyph.
 * Each call is snapped to the cell it lands in and recorded in a grid that the
 * terminal writer then diffs and emits as 24-bit ANSI.
 */

/** Cell background left to the terminal's own colour. */
export const DEFAULT_BG = -1;
const DOT_GLYPH = "·";

type Rgba = [rgb: number, alpha: number];

const colorCache = new Map<string, Rgba>();

/** Parses the three forms the renderer emits: #rrggbb, rgb(...), rgba(...). */
function parseColor(s: string): Rgba {
  let c = colorCache.get(s);
  if (c) return c;
  if (s[0] === "#") {
    c = [parseInt(s.slice(1, 7), 16), 1];
  } else {
    const [r, g, b, a = 1] = s.slice(s.indexOf("(") + 1, -1).split(",").map(Number);
    c = [(r << 16) | (g << 8) | b, a];
  }
  if (colorCache.size > 50_000) colorCache.clear();
  colorCache.set(s, c);
  return c;
}

/** Source-over onto an opaque colour; the default background counts as black. */
function over(rgb: number, alpha: number, under: number) {
  if (alpha >= 1) return rgb;
  const u = under === DEFAULT_BG ? 0 : under;
  const mix = (shift: number) =>
    Math.round(((rgb >> shift) & 255) * alpha + ((u >> shift) & 255) * (1 - alpha));
  return (mix(16) << 16) | (mix(8) << 8) | mix(0);
}

export class CellCanvas {
  // Written by AsciiRenderer.resize(); unused here.
  width = 0;
  height = 0;

  cols = 0;
  rows = 0;
  /** Virtual pixel size of one cell, matching the renderer's cw/ch. */
  cw = 1;
  ch = 1;
  glyph: string[] = [];
  fg = new Int32Array(0);
  bg = new Int32Array(0);

  private ctx = new CellContext(this);

  getContext() {
    return this.ctx;
  }

  resize(cols: number, rows: number, cw: number, ch: number) {
    this.cols = cols;
    this.rows = rows;
    this.cw = cw;
    this.ch = ch;
    this.glyph = new Array(cols * rows).fill(" ");
    this.fg = new Int32Array(cols * rows);
    this.bg = new Int32Array(cols * rows).fill(DEFAULT_BG);
  }

  clear() {
    this.glyph.fill(" ");
    this.fg.fill(0);
    this.bg.fill(DEFAULT_BG);
  }

  cellAt(x: number, y: number) {
    const c = Math.floor(x / this.cw);
    const r = Math.floor(y / this.ch);
    if (c < 0 || r < 0 || c >= this.cols || r >= this.rows) return -1;
    return r * this.cols + c;
  }
}

class CellContext {
  // Set by the renderer, irrelevant to a character grid.
  font = "";
  textAlign = "";
  textBaseline = "";

  private style = "#000";
  private rgb = 0;
  private alpha = 1;

  constructor(private grid: CellCanvas) {}

  get fillStyle() {
    return this.style;
  }

  set fillStyle(s: string) {
    this.style = s;
    [this.rgb, this.alpha] = parseColor(s);
  }

  setTransform() {}

  /** The renderer's only drawImage blits its blank background: start a fresh frame. */
  drawImage() {
    this.grid.clear();
  }

  fillRect(x: number, y: number, w: number, h: number) {
    const g = this.grid;
    const i = g.cellAt(x + w / 2, y + h / 2);
    if (i < 0) return;
    if (w >= g.cw / 2) {
      // Shark skin: the block behind the glyph.
      g.bg[i] = over(this.rgb, this.alpha, g.bg[i]);
      g.glyph[i] = " ";
    } else {
      g.fg[i] = over(this.rgb, this.alpha, g.bg[i]);
      g.glyph[i] = DOT_GLYPH;
    }
  }

  fillText(text: string, x: number, y: number) {
    const g = this.grid;
    const i = g.cellAt(x, y);
    if (i < 0) return;
    g.fg[i] = over(this.rgb, this.alpha, g.bg[i]);
    g.glyph[i] = text;
  }
}

/** Absorbs everything: the renderer's offscreen background canvas only draws grid lines. */
export const nullCanvas = {
  width: 0,
  height: 0,
  getContext: () => new Proxy({}, { get: () => () => {}, set: () => true }),
};
