// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The UI DRY gate (docs/design/ui-system.md §9). Each cross-cutting concern
// has ONE home under web/; this script fails a file that re-implements it
// elsewhere. Zero dependencies, plain node.
//
//   node scripts/check-ui-dry.mjs          # every finding fails
//
// Blocking everywhere under `src/` and `demo/` (since #350; the advisory
// mode that eased the M10 migration is retired — every organism is on the
// primitives). The allow-list holds the two reasoned exceptions in
// `swath-map.ts`; a stale entry (one that no longer matches) fails too, so
// the escape hatch can only shrink.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const SCANNED = ["src", "demo"];
const EXTENSIONS = new Set([".ts", ".css", ".html"]);

/**
 * Rules: `homes` lists the files allowed to match — usually one, and never
 * more than the design says a value may live in. `tests: false` skips
 * `*.test.ts` — a test may legitimately pin a computed `rgb(…)` from
 * `getComputedStyle`; production code may not write one.
 */
const RULES = [
  {
    id: "color-literal",
    // Two homes since #389: the palette, and the high-contrast theme that
    // narrows it. A theme IS a set of colour literals — that is what a theme
    // is — so the rule is "colours live in a palette file", not "colours live
    // in exactly one file".
    homes: ["src/ui/tokens.css", "src/ui/theme-high-contrast.css"],
    tests: false,
    // A hex colour: one with a letter anywhere (`#4ade80`, `#fff`) unless it
    // is a private field (`#fade =`, `#bed(`, `#feed:`, `#cab in`); an all-digit one
    // (`#210`, also an issue number) only in CSS value position (`: #210`).
    //
    // `;` is NOT in the exclusion list, though it was until #389: excluding it
    // made `color: #ff00aa;` — the ordinary shape of a CSS colour declaration —
    // invisible to this gate, while the all-digit `#000000;` was still caught.
    // Verified when it was removed: the theme file added in that PR was the
    // only place in `src/` and `demo/` the corrected pattern newly matched.
    pattern:
      /(?<![\w#.&])#(?=[0-9a-f]*[a-f])(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})\b(?!\s*(?:=|\(|:|in\b))|(?<=:\s*)#(?:\d{3,4}|\d{6}|\d{8})\b|\b(?:rgb|hsl)a?\(/gi,
  },
  {
    id: "font-literal",
    homes: ["src/ui/tokens.css"],
    tests: false,
    pattern:
      /\bfont(?:-family)?\s*:\s*(?!inherit\b|var\(|initial\b|unset\b)[^;"`}]*(?:sans-serif|serif|monospace|system-ui)/g,
  },
  {
    id: "style-injection",
    homes: ["src/ui/element.ts"],
    tests: true,
    pattern: /\binjectStyles\b|\bSTYLE_ELEMENT_ID\b|customElements\.define\(/g,
  },
  {
    id: "custom-event",
    homes: ["src/ui/events.ts"],
    tests: true,
    pattern: /new CustomEvent\(/g,
  },
  {
    id: "bare-fetch",
    homes: ["src/api.ts"],
    tests: true,
    pattern: /(?<![\w.#])fetch\(/g,
  },
  {
    id: "ui-reaches-out",
    homes: [],
    tests: true,
    only: /^src\/ui\//,
    // A value import creates the dependency; `import type` is erased and
    // only names an organism's detail shape in the event catalog.
    pattern: /^import\s+(?!type\b)[^;]*?from\s+["']\.\.\//gm,
  },
];

/** `{ file, rule, reason }` — every entry must still match something. */
const ALLOW = [
  {
    file: "src/swath-map.ts",
    rule: "style-injection",
    reason: "MapLibre's own stylesheet is self-injected once per document; it is not Swath chrome",
  },
  {
    file: "src/swath-map.ts",
    rule: "bare-fetch",
    reason: "the basemap style cache fetches a foreign URL, outside the Swath API seam",
  },
];

function* walk(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      yield* walk(path);
    } else if (EXTENSIONS.has(path.slice(path.lastIndexOf(".")))) {
      yield path;
    }
  }
}

function lineOf(text, index) {
  let line = 1;
  for (let i = 0; i < index; i += 1) {
    if (text.charCodeAt(i) === 10) {
      line += 1;
    }
  }
  return line;
}

const findings = [];
const allowHits = new Map(ALLOW.map((entry) => [`${entry.file}::${entry.rule}`, 0]));

for (const dir of SCANNED) {
  for (const path of walk(join(ROOT, dir))) {
    const file = relative(ROOT, path).split(sep).join("/");
    const isTest = file.endsWith(".test.ts");
    const text = readFileSync(path, "utf8");
    for (const rule of RULES) {
      if (
        rule.homes.includes(file) ||
        (isTest && !rule.tests) ||
        (rule.only && !rule.only.test(file))
      ) {
        continue;
      }
      for (const match of text.matchAll(rule.pattern)) {
        const key = `${file}::${rule.id}`;
        if (allowHits.has(key)) {
          allowHits.set(key, allowHits.get(key) + 1);
          continue;
        }
        findings.push({ file, rule: rule.id, line: lineOf(text, match.index), text: match[0] });
      }
    }
  }
}

const stale = ALLOW.filter((entry) => allowHits.get(`${entry.file}::${entry.rule}`) === 0);

const out = process.stdout;
for (const f of findings) {
  out.write(`FAIL  ${f.file}:${f.line}  ${f.rule}  ${JSON.stringify(f.text)}\n`);
}
for (const entry of stale) {
  out.write(
    `FAIL  stale allow-list entry: ${entry.file} / ${entry.rule} no longer matches — remove it\n`,
  );
}

const byFile = new Set(findings.map((f) => f.file)).size;
out.write(
  `check-ui-dry: ${findings.length} finding(s) in ${byFile} file(s); ${stale.length} stale allow-list\n`,
);
process.exit(findings.length + stale.length > 0 ? 1 : 0);
