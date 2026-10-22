// Generates the README artwork for NEX (light and dark variants).
// Usage: node tools/readme-art.js docs/assets
const fs = require('fs');
const path = require('path');
const out = process.argv[2];

const SANS = "'Segoe UI', 'Helvetica Neue', Helvetica, Arial, sans-serif";
const MONO = "'JetBrains Mono', 'Cascadia Code', Consolas, Menlo, monospace";

const themes = {
  dark: {
    bg1: '#070B18', bg2: '#101934', grid: '#FFFFFF', gridOp: 0.045,
    title1: '#FFFFFF', title2: '#B9C6FF', text: '#A9B4D6', chipBg: '#151F3D',
    chipStroke: '#2A3866', chipText: '#D6DEFF', glowA: '#7C5CFF', glowB: '#00C2FF',
    box: '#111A33', boxStroke: '#2B3A6B', boxTitle: '#F2F5FF', boxSub: '#8F9CC6', arrow: '#5B6BA3',
  },
  light: {
    bg1: '#F7F8FF', bg2: '#E9EEFF', grid: '#1B2559', gridOp: 0.05,
    title1: '#0B1230', title2: '#4B3FD1', text: '#47507A', chipBg: '#FFFFFF',
    chipStroke: '#D3DAF5', chipText: '#27305A', glowA: '#8B6CFF', glowB: '#22C7FF',
    box: '#FFFFFF', boxStroke: '#D3DAF5', boxTitle: '#141B3D', boxSub: '#6A739B', arrow: '#A3ADD6',
  },
};

// The code window is dark in both themes.
const code = {
  bg: '#0B1226', stroke: '#26325C', bar: '#111A35', file: '#7F8BB5',
  kw: '#FF7AB2', fn: '#7EE0FF', str: '#A8E07A', punct: '#C9D2F2', interp: '#FFC66D', plain: '#E8ECFF', comment: '#5F6B96',
};

function esc(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function mark(x, y, size, t) {
  // Hexagon badge with an "N" monogram.
  const s = size / 100;
  const p = (px, py) => `${(x + px * s).toFixed(1)},${(y + py * s).toFixed(1)}`;
  const hex = [p(50, 2), p(93, 26), p(93, 74), p(50, 98), p(7, 74), p(7, 26)].join(' ');
  const n = `M${p(32, 70)} L${p(32, 30)} L${p(68, 70)} L${p(68, 30)}`;
  return `
  <polygon points="${hex}" fill="url(#markFill)" />
  <polygon points="${hex}" fill="none" stroke="#FFFFFF" stroke-opacity="0.25" stroke-width="${1.5 * s}" />
  <path d="${n}" fill="none" stroke="#FFFFFF" stroke-width="${9 * s}" stroke-linecap="round" stroke-linejoin="round" />`;
}

function chip(x, y, label, t) {
  const w = Math.round(label.length * 8.6 + 34);
  return `
  <g>
    <rect x="${x}" y="${y}" width="${w}" height="34" rx="17" fill="${t.chipBg}" stroke="${t.chipStroke}" />
    <circle cx="${x + 17}" cy="${y + 17}" r="4" fill="url(#accent)" />
    <text x="${x + 28}" y="${y + 22.5}" font-family="${SANS}" font-size="15" font-weight="600" fill="${t.chipText}">${esc(label)}</text>
  </g>`;
  // width returned via closure below
}
function chipWidth(label) {
  return Math.round(label.length * 8.6 + 34);
}

function codeLines() {
  // Each line is a list of [color, text] segments.
  const K = code.kw, F = code.fn, S = code.str, P = code.punct, I = code.interp, W = code.plain, C = code.comment;
  return [
    [[C, '// hello.nex']],
    [[K, 'app'], [W, ' '], [F, 'Hello'], [P, ' {']],
    [[W, '    '], [K, 'let'], [W, ' langs '], [P, '= ['], [S, '"NEX"'], [P, ', '], [S, '"Rust"'], [P, ']']],
    [[W, '    '], [K, 'for'], [W, ' lang '], [K, 'in'], [W, ' langs.'], [F, 'sorted'], [P, '() {']],
    [[W, '        '], [K, 'say'], [W, ' '], [S, '"Hello, '], [I, '{lang}'], [S, '!"']],
    [[P, '    }']],
    [[P, '}']],
  ];
}

function banner(t) {
  const W = 1280, H = 420;
  const cx = 760, cy = 92, cw = 440, ch = 252;
  const lines = codeLines().map((segs, i) => {
    const spans = segs.map(([c, s]) => `<tspan fill="${c}">${esc(s)}</tspan>`).join('');
    return `<text x="${cx + 26}" y="${cy + 84 + i * 26}" font-family="${MONO}" font-size="16" xml:space="preserve">${spans}</text>`;
  }).join('\n    ');

  const chips = ['Statically typed', 'Verified bytecode', 'Written in Rust'];
  let chipX = 80;
  const chipSvg = chips.map((c) => {
    const s = chip(chipX, 300, c, t);
    chipX += chipWidth(c) + 12;
    return s;
  }).join('');

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-labelledby="title desc">
  <title id="title">NEX programming language</title>
  <desc id="desc">NEX: a fast, safe and simple programming language by NexDeve. Statically typed, compiled to verified bytecode, written in Rust.</desc>
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="${t.bg1}" />
      <stop offset="1" stop-color="${t.bg2}" />
    </linearGradient>
    <linearGradient id="accent" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="${t.glowA}" />
      <stop offset="1" stop-color="${t.glowB}" />
    </linearGradient>
    <linearGradient id="markFill" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#7C5CFF" />
      <stop offset="1" stop-color="#00B8F0" />
    </linearGradient>
    <linearGradient id="titleGrad" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="${t.title1}" />
      <stop offset="1" stop-color="${t.title2}" />
    </linearGradient>
    <radialGradient id="glowA" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="${t.glowA}" stop-opacity="0.45" />
      <stop offset="1" stop-color="${t.glowA}" stop-opacity="0" />
    </radialGradient>
    <radialGradient id="glowB" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="${t.glowB}" stop-opacity="0.35" />
      <stop offset="1" stop-color="${t.glowB}" stop-opacity="0" />
    </radialGradient>
    <pattern id="grid" width="40" height="40" patternUnits="userSpaceOnUse">
      <path d="M40 0H0V40" fill="none" stroke="${t.grid}" stroke-opacity="${t.gridOp}" stroke-width="1" />
    </pattern>
    <clipPath id="frame"><rect width="${W}" height="${H}" rx="24" /></clipPath>
  </defs>

  <g clip-path="url(#frame)">
    <rect width="${W}" height="${H}" fill="url(#bg)" />
    <rect width="${W}" height="${H}" fill="url(#grid)" />
    <circle cx="1040" cy="60" r="330" fill="url(#glowA)" />
    <circle cx="820" cy="430" r="300" fill="url(#glowB)" />
    <circle cx="120" cy="40" r="220" fill="url(#glowA)" opacity="0.5" />

    ${mark(80, 84, 104, t)}
    <text x="206" y="172" font-family="${SANS}" font-size="112" font-weight="800" letter-spacing="6" fill="url(#titleGrad)">NEX</text>
    <text x="80" y="242" font-family="${SANS}" font-size="27" font-weight="600" fill="${t.title1}">A fast, safe and simple programming language</text>
    <text x="80" y="274" font-family="${SANS}" font-size="19" fill="${t.text}">by NexDeve · nexdeve.com</text>
    ${chipSvg}

    <g>
      <rect x="${cx + 6}" y="${cy + 10}" width="${cw}" height="${ch}" rx="16" fill="#000000" opacity="0.18" />
      <rect x="${cx}" y="${cy}" width="${cw}" height="${ch}" rx="16" fill="${code.bg}" stroke="${code.stroke}" />
      <path d="M${cx} ${cy + 16} a16 16 0 0 1 16 -16 h${cw - 32} a16 16 0 0 1 16 16 v26 h-${cw} z" fill="${code.bar}" />
      <circle cx="${cx + 24}" cy="${cy + 21}" r="6" fill="#FF5F57" />
      <circle cx="${cx + 44}" cy="${cy + 21}" r="6" fill="#FEBC2E" />
      <circle cx="${cx + 64}" cy="${cy + 21}" r="6" fill="#28C840" />
      <text x="${cx + cw / 2}" y="${cy + 26}" text-anchor="middle" font-family="${MONO}" font-size="13" fill="${code.file}">hello.nex</text>
    ${lines}
    </g>
  </g>
</svg>
`;
}

function pipeline(t) {
  const steps = [
    ['Source', '.nex file'],
    ['Lexer', 'tokens'],
    ['Parser', 'syntax tree'],
    ['Checker', 'names & types'],
    ['Compiler', 'bytecode'],
    ['Verifier', 'safety checks'],
    ['VM', 'runs your app'],
  ];
  const W = 1280, H = 190, bw = 148, bh = 92, gap = (W - 80 - steps.length * bw) / (steps.length - 1);
  const y = 50;
  let svg = '';
  steps.forEach(([title, sub], i) => {
    const x = 40 + i * (bw + gap);
    const highlight = i === 0 || i === steps.length - 1;
    svg += `
  <g>
    <rect x="${x}" y="${y}" width="${bw}" height="${bh}" rx="14" fill="${t.box}" stroke="${highlight ? 'url(#accent)' : t.boxStroke}" stroke-width="${highlight ? 2 : 1.2}" />
    <text x="${x + bw / 2}" y="${y + 40}" text-anchor="middle" font-family="${SANS}" font-size="18" font-weight="700" fill="${t.boxTitle}">${esc(title)}</text>
    <text x="${x + bw / 2}" y="${y + 64}" text-anchor="middle" font-family="${SANS}" font-size="13.5" fill="${t.boxSub}">${esc(sub)}</text>
  </g>`;
    if (i < steps.length - 1) {
      const ax = x + bw + 6, bx = x + bw + gap - 6, ay = y + bh / 2;
      svg += `
  <path d="M${ax.toFixed(1)} ${ay} H${(bx - 6).toFixed(1)}" stroke="${t.arrow}" stroke-width="2" stroke-linecap="round" />
  <path d="M${(bx - 10).toFixed(1)} ${ay - 5} L${bx.toFixed(1)} ${ay} L${(bx - 10).toFixed(1)} ${ay + 5}" fill="none" stroke="${t.arrow}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />`;
    }
  });
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-labelledby="title">
  <title id="title">How NEX runs a program: source, lexer, parser, checker, compiler, verifier, VM</title>
  <defs>
    <linearGradient id="accent" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="${t.glowA}" />
      <stop offset="1" stop-color="${t.glowB}" />
    </linearGradient>
  </defs>
  ${svg}
  <text x="${W / 2}" y="${y + bh + 34}" text-anchor="middle" font-family="${SANS}" font-size="14" fill="${t.boxSub}">Nothing runs until every stage passes. Errors point to the exact line and column.</text>
</svg>
`;
}

fs.mkdirSync(out, { recursive: true });
for (const [name, t] of Object.entries(themes)) {
  fs.writeFileSync(path.join(out, `banner-${name}.svg`), banner(t));
  fs.writeFileSync(path.join(out, `pipeline-${name}.svg`), pipeline(t));
}
console.log('wrote', fs.readdirSync(out).filter((f) => f.endsWith('.svg')).join(', '));
