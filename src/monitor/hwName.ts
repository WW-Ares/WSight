/**
 * Hardware names, shortened - and nothing else.
 *
 * Why this is its own module, with no imports: WSight ships to whatever machine
 * a stranger happens to own, and the strings we show come straight out of
 * vendor firmware (SMBIOS) where there is no standard at all. The same i5 can
 * be spelled four ways on a Lenovo, an HP, an ASUS and a whitebox. So the rules
 * have to be pressured against a wide corpus, not just tuned until this
 * developer's own machine looks right - and that corpus lives in
 * `.verify/hw_short_test.mjs`, which imports this file directly. One
 * implementation, no second copy to drift.
 *
 * The budget is a real number. The gauge caption sits in a four-column grid at
 * 300 design px wide with 6 px side padding and a 2 px gutter, so each caption
 * gets (300 - 12 - 6) / 4 = 70.5 design px. Measure in *design* px, never
 * physical px: the panel renders at 420 physical px on this display, but the
 * whole stage is scaled by the same 1.4 factor, so the percentages agree while
 * the raw numbers do not. Comparing a physical column width against a design
 * font size is how an earlier pass talked itself into "this fits" and shipped a
 * truncated card. Calibration: `.verify/calib_column.py`.
 *
 * Every export returns the *most informative* spelling that fits, by walking a
 * ladder of progressively harsher reductions and stopping at the first rung
 * that fits. Two consequences worth stating:
 *
 *   - Nothing is ever silently truncated by these rules. If even the harshest
 *     rung overflows, we return it anyway and the `<FitText>` wrapper shrinks
 *     the type a little instead - and only if that hits its own floor does an
 *     ellipsis appear, with the full string on the tooltip.
 *   - The reductions are *rules*, not a lookup table. A vendor abbreviation
 *     table (ASUS's `M12H` for MAXIMUS XII HERO, say) is real - it is what ASUS
 *     names its own BIOS files - but no two vendors share a convention, no
 *     reusable table exists, and there are thousands of boards with more every
 *     year. `M12H` is also opaque to anyone outside the enthusiast forums,
 *     whereas a slightly smaller "MAXIMUS XII" still reads.
 */

/**
 * A four-column caption's budget, in design px. Three columns get 94 design px
 * instead, which is why nothing here may hardcode "this string is fine" - the
 * caller measures, and `<FitText>` re-measures when the layout changes.
 */
export const COL_PX = 70;

/**
 * Character advances at 10 px, calibrated against a real screenshot
 * (`i9-10900K` measures 44.3 design px, `RTX 2080 Ti` measures 50.0).
 *
 * The model reads about 5% wide on average. That bias is deliberate: guessing
 * wide costs a slightly shorter caption, guessing narrow ships a truncation.
 */
export function textWidth(s: string): number {
  let w = 0;
  for (const ch of s) {
    const c = ch.codePointAt(0) ?? 0;
    if (c > 0x2e80) w += 10.0; // CJK is full width
    else if (ch === " ") w += 3.0;
    else if (/[iljI.,:;'|!()[\]/]/.test(ch)) w += 2.6;
    else if (ch >= "0" && ch <= "9") w += 5.3; // tabular-nums
    else if (ch >= "A" && ch <= "Z") w += 6.2;
    else if (ch >= "a" && ch <= "z") w += 5.2;
    else w += 3.3; // - + ℃ and friends
  }
  return w;
}

/** Does this string fit a four-column caption at full size? */
export function fitsText(s: string): boolean {
  return textWidth(s) <= COL_PX;
}

/** Collapse the whitespace a replacement always leaves behind. */
function tidy(s: string): string {
  return s.replace(/[\s,;·]+$/g, "").replace(/\s+/g, " ").trim();
}

/**
 * Take the first rung that fits; if none does, take the last one and let
 * `<FitText>` deal with it. Returning `""` means "there is nothing honest to
 * show here" and callers render the `--` placeholder.
 */
export function pickLevel(levels: string[]): string {
  const seen: string[] = [];
  for (const l of levels) {
    const t = tidy(l);
    if (!t || seen.includes(t)) continue;
    seen.push(t);
    if (fitsText(t)) return t;
  }
  return seen.length ? seen[seen.length - 1] : "";
}

// ============================================================== CPU

/**
 * Core-count words that sometimes precede `-Core` (`FX-8350 Eight-Core`).
 * The Arabic-number form is handled by the same rule.
 */
const CPU_CORE_WORDS =
  "Single|Dual|Tri|Quad|Hexa|Six|Octa|Eight|Deca|Ten|Dodeca|Twelve|Hexadeca|Sixteen";

/**
 * Strip a CPU brand string down to the model. This is the *noise* pass - it
 * only ever removes words that carry no model identity, so the result is never
 * a guess at what the part is.
 */
function cpuDenoise(brand: string): string {
  if (!brand) return "";
  // `(R) (TM) (C)` are removed in place rather than replaced by a space:
  // `FX(tm)-8350` would otherwise split into `FX -8350`.
  let s = brand.replace(/\((?:R|TM|C)\)/gi, "");
  // Everything after a spaced hyphen is filler - Snapdragon's
  // `X Elite - X1E80100 - Qualcomm Oryon CPU`. Hyphens inside a model
  // (`i9-10900K`) have no surrounding spaces and survive.
  s = s.replace(/\s+-\s+.*$/, " ");
  // The rated clock is already on the line above the caption, and it is wrong
  // the moment the part boosts.
  s = s.replace(/@.*$/, " ");
  // Integrated-graphics tails: the model already says which APU this is.
  s = s.replace(/\s+(?:with|w\/)\s+.*$/i, " ");
  s = s.replace(/\b\d+\s+Compute Cores?\b.*$/i, " ");
  // Generation prefixes - `13th Gen Intel(R) Core(TM) i5-13400F`. Without this
  // the caption is 87 px against a 70 px budget, every modern Intel machine.
  s = s.replace(/\b\d{1,2}(?:st|nd|rd|th)\s*Gen\.?\b/gi, " ");
  s = s.replace(/\b(?:Intel|AMD|Genuine)\b/gi, " ");
  s = s.replace(/\b(?:CPU|Processor|APU|SoC)\b/gi, " ");
  // `Core i9-10900K` and `Core Ultra 7 155H` read better without `Core`;
  // `Core 2 Duo` is nothing but that word, so it stays.
  s = s.replace(/\bCore\s+(?=i\d|Ultra)/gi, "");
  s = s.replace(new RegExp(`\\b(?:\\d+|${CPU_CORE_WORDS})[\\s-]Core\\b`, "gi"), " ");
  // `A8-7600 Radeon R7, 10 Compute Cores…` / `Radeon HD 8570D` - the model
  // already identifies the APU.
  const noGpu = s.replace(/\bRadeon\s+(?:R\d|HD|Vega|Graphics)\b.*$/i, " ").trim();
  if (noGpu) s = noGpu;
  // Tier words that rank a part without naming it. Intel writes `Pentium
  // Gold G5400` (94 px); Intel also writes `Pentium G5400` (70 px).
  s = s.replace(/\b(?:Gold|Silver)\b(?=\s+\S*\d)/gi, " ");
  return tidy(s);
}

/** The ladder of CPU captions, most informative first. */
export function cpuCaptionLevels(brand: string): string[] {
  const base = cpuDenoise(brand);
  if (!base) return [];
  const levels = [base];
  // Cumulative rewrites. Each is a rule about information, not a table of
  // part numbers, so a chip nobody has shipped yet still gets shortened.
  const steps: ((s: string) => string)[] = [
    // Roman-numeral generations where the model number carries the identity:
    // `Athlon II X4 640` -> `Athlon X4 640` (71 px -> 63 px).
    (s) => s.replace(/\b(?:II|III|IV)\b(?=\s)/g, " "),
    // SKU words that sit behind a family name: `Snapdragon X Elite` -> the
    // family is `Snapdragon X`, `Elite`/`Plus` only rank it.
    (s) => s.replace(/\b(?:Elite|Plus)\b/g, " "),
    // Stepping suffix - the last thing worth losing: `Xeon E5-2670 v3` ->
    // `Xeon E5-2670`, because `Xeon` tells a lay reader far more than `v3`.
    (s) => s.replace(/\s+v\d+$/i, " "),
    // AMD's own shorthand, used in its own marketing: `Ryzen 9 7950X3D` ->
    // `R9 7950X3D`. `Ryzen AI 9 HX 370` -> `R9 HX 370`.
    (s) => s.replace(/\bRyzen\s+AI\s+(\d)\b/gi, "R$1").replace(/\bRyzen\s+(\d)\b/gi, "R$1"),
    // `Threadripper` is officially abbreviated `TR` by AMD.
    (s) =>
      s.replace(/\bRyzen\s+Threadripper\b/gi, "TR").replace(/\bThreadripper\b/gi, "TR"),
    // Last resort: keep whatever token carries a digit, which is the model.
    (s) => (s.match(/[A-Za-z]*\d[\w.-]*/) || [s])[0],
  ];
  let cur = base;
  for (const step of steps) {
    const next = tidy(step(cur));
    if (next && next !== cur) levels.push(next);
    cur = next;
  }
  return levels;
}

export function shortCpuModel(brand: string): string {
  return pickLevel(cpuCaptionLevels(brand));
}

// ============================================================== GPU

/**
 * Names that mean "this is not a graphics card".
 *
 * Public-release machines routinely have several. This developer's own box
 * carries four: two `Microsoft Remote Display Adapter`, a GameViewer virtual
 * display and an Oray (Sunlogin) indirect device. Printing those in the GPU
 * column is worse than printing nothing, and on a machine whose only real GPU
 * has no driver, `Microsoft Basic Display Adapter` is the honest answer to
 * "which GPU" - namely, unknown.
 */
const GPU_VIRTUAL_RE = new RegExp(
  [
    "virtual",
    "indirect",
    "idd",
    "remote display",
    "basic display",
    "basic render",
    "hyper-v",
    "vmware",
    "virtualbox",
    "qxl",
    "virtio",
    "svga",
    "mirror",
    "vga graphics adapter",
    "display only",
    "splashtop",
    "parsec",
    "todesk",
    "oray",
    "sunlogin",
    "anydesk",
    "rustdesk",
    "gameviewer",
    "citrix",
    "meta virtual",
    "usb mobile monitor",
    "astral",
    "dameware",
    "windows virtual display",
    "amazon vdi",
    "ngfx",
  ].join("|"),
  "i",
);

/** Tail modifiers that add nothing once the model number is present. */
const GPU_TAIL_NOISE = new Set([
  "GPU",
  "GRAPHICS",
  "FAMILY",
  "SERIES",
  "ADAPTER",
  "DISPLAY",
  "CONTROLLER",
  "VIDEO",
]);

function gpuDenoise(name: string): string {
  if (!name) return "";
  if (GPU_VIRTUAL_RE.test(name)) return "";
  const base = tidy(name.replace(/\((?:R|TM|C)\)/gi, " "));

  const dropVendor = (t: string) => tidy(t.replace(/\b(?:NVIDIA|AMD|ATI|Intel)\b/gi, " "));
  const dropFamily = (t: string) => tidy(t.replace(/\b(?:GeForce|Radeon)\b/gi, " "));
  const tokens = (t: string) => t.split(" ").filter(Boolean);
  const allGeneric = (t: string) => {
    const tk = tokens(t);
    return !tk.length || tk.every((x) => GPU_TAIL_NOISE.has(x.toUpperCase()));
  };

  const preNoise = dropFamily(dropVendor(base));

  // Trim modifiers only from the tail, and only while 3+ tokens remain: in
  // `UHD Graphics 630` the `Graphics` is interior and carries the meaning
  // (`UHD 630` is the same part; `Graphics 630` is nothing).
  let s = preNoise;
  for (;;) {
    const tk = tokens(s);
    if (tk.length < 3 || !GPU_TAIL_NOISE.has(tk[tk.length - 1].toUpperCase())) break;
    s = tk.slice(0, -1).join(" ");
  }
  // Trimming to a single token (`Arc`, `UHD`) went too far - step back.
  if (tokens(s).length < 2 && tokens(preNoise).length > 1) s = preNoise;
  // Still all filler (`AMD Radeon Graphics` -> `Graphics`): prefer
  // `Radeon Graphics`, and if that is filler too, the original name.
  if (allGeneric(s)) {
    const withFamily = dropVendor(base);
    s = !allGeneric(withFamily) ? withFamily : base;
  }
  return s;
}

/**
 * The ladder of GPU captions, most informative first.
 *
 * Vendor words go unconditionally; family words only for NVIDIA and AMD, whose
 * models stand alone (`RTX 2080 Ti`, `RX 580`). Intel keeps `Arc` and `Iris`,
 * because `A770` and `Xe` on their own read like a typo rather than a model.
 */
export function gpuCaptionLevels(name: string): string[] {
  const base = gpuDenoise(name);
  if (!base) return [];
  const levels = [base];
  const steps: ((s: string) => string)[] = [
    // `UHD Graphics 630` -> `UHD 630`; `HD Graphics 530` -> `HD 530`;
    // `Radeon Graphics` -> `Radeon`. `Graphics` is pure filler - Intel and AMD
    // both spend it, and neither means anything by it.
    (s) => s.replace(/\b(?:Graphics|GPU)\b/gi, " "),
    // `RTX 4060 Laptop GPU` -> `RTX 4060`. Mobile/desktop is not a model.
    (s) => s.replace(/\b(?:Laptop|Mobile|Max-Q|Desktop)\b/gi, " "),
    // `GTX 1660 SUPER` -> `GTX 1660S`. The `S` has to survive: a plain
    // `GTX 1660` is a different card, so dropping `SUPER` outright would lie.
    (s) => s.replace(/\s+SUPER\b/gi, "S"),
    // Last resort: the model number and its qualifier.
    (s) => s.split(" ").slice(-2).join(" "),
  ];
  let cur = base;
  for (const step of steps) {
    const next = tidy(step(cur));
    if (next && next !== cur) levels.push(next);
    cur = next;
  }
  return levels;
}

export function shortGpuModel(name: string): string {
  return pickLevel(gpuCaptionLevels(name));
}

// ============================================================== board

/** Firmware placeholders. Printing them is worse than printing `--`. */
const BOARD_JUNK = new Set([
  "DEFAULT STRING",
  "TO BE FILLED BY O.E.M.",
  "TO BE FILLED BY OEM",
  "SYSTEM PRODUCT NAME",
  "BASE BOARD PRODUCT NAME",
  "SYSTEM VERSION",
  "BASE BOARD VERSION",
  "NOT SPECIFIED",
  "NOT AVAILABLE",
  "UNKNOWN",
  "INVALID",
  "OEM",
  "O.E.M.",
  "N/A",
  "NA",
  "NONE",
  "X.X",
  "A M I",
  "TYPE2 - BOARD PRODUCT NAME",
  "ASUSTEK COMPUTER INC.",
]);

/**
 * Whole-machine vendors put a *part number* in the board field, not a model:
 * Dell's `0NW6H5`, HP's `8643`, Lenovo's `3141`. Nobody can read those, and a
 * `--` says "we do not have a board name" more honestly than the number does.
 * Real board names always carry a letter pair plus a chipset number
 * (`B550M`, `X570`, `M5A97`), so a bare four-digit string is not one.
 */
const OEM_BOARD_CODE = /^(?:\d{4}|0[0-9A-Z]{4,6})$/;

/** Leading series words. Stripped second - after the firmware noise. */
const BOARD_SERIES = [
  "ROG STRIX",
  "TUF GAMING",
  "PRO WS",
  "PHANTOM GAMING",
  "STEEL LEGEND",
  "PRO RS",
  "PROART",
  "PRIME",
  "ROG",
  "TUF",
  "WS",
  "MAG",
  "MPG",
  "MEG",
  "AORUS",
  "AERO",
  "TAICHI",
];

/** Trailing marketing noise: connectivity, memory generation, revisons. */
const BOARD_TAIL_NOISE = [
  "WI-FI 6E",
  "WI-FI 7",
  "WI-FI 6",
  "WI-FI",
  "WIFI6E",
  "WIFI6",
  "WIFI7",
  "WIFI",
  "WLAN",
  "BT",
  "DDR5",
  "DDR4",
  "DDR3",
  "D5",
  "D4",
  "CSM",
  "R2.0",
  "R1.0",
  "REV",
  "A-RGB",
  "ARGB",
  "AC",
  "AX",
  "MAX",
];

/**
 * Tier words: they rank a board inside its series without naming it. Dropped
 * one at a time, so `X570 AORUS PRO` becomes `X570 AORUS` - and stops there,
 * because `AORUS` is the vendor and losing it would make the slab a Gigabyte
 * and an ASUS look identical.
 *
 * The list spans the five vendors that ship most consumer boards (ASUS, Gigabyte,
 * MSI, ASRock, Biostar) because a public build meets all of them. Hyphenated
 * suffixes are handled separately, which is why the bare model suffixes
 * (`DS3H`, `K`) are here only as whole tokens.
 */
const BOARD_TAIL_TIER = [
  // ASUS and ASRock
  "TOMAHAWK",
  "BAZOOKA",
  "MORTAR",
  "GODLIKE",
  "CREATOR",
  "EXTREME4",
  "EXTREME",
  "FORMULA",
  "LIVEMIXER",
  "VELOCITA",
  "RIPTIDE",
  "VALKYRIE",
  "SILVER",
  "RACING",
  "UNIFY",
  "CARBON",
  "MASTER",
  "XTREME",
  "VISION",
  "NOVA",
  "STRIX",
  "HERO",
  "ELITE",
  "PLUS",
  "ULTRA",
  "APEX",
  "GENE",
  "PRO4",
  "ACE",
  "EDGE",
  "GAMING",
  "IMPACT",
  "PRO",
  "MAX",
  // Bare model suffixes (MSI's `-VDH`, Gigabyte's `DS3H`, ASUS's `K`)
  "VDH",
  "VH",
  "HDV",
  "HD3",
  "D3H",
  "D3V",
  "DS3H",
  "D2H",
  "S2H",
  "S3H",
  "K",
];

function isJunkBoard(v: string): boolean {
  const t = (v || "").trim().toUpperCase();
  return !t || BOARD_JUNK.has(t);
}

/** Firmware noise: parenthesised suffixes, MSI's `(MS-7D42)`, tail labels. */
function boardNoise(name: string): string {
  if (isJunkBoard(name)) return "";
  let s = name.toUpperCase();
  s = s.replace(/\((?:REV[^)]*|V?\d+\.\d+)\)/gi, " ");
  s = s.replace(/\(MS-[0-9A-F]+\)/gi, " ");
  s = s.replace(/\s*\([^)]*\)\s*/g, " ");
  s = tidy(s);
  for (let changed = true; changed; ) {
    changed = false;
    for (const w of BOARD_TAIL_NOISE) {
      const re = new RegExp(`(?:^|\\s)${escapeRe(w)}\\s*$`, "i");
      if (re.test(s)) {
        s = tidy(s.replace(re, " "));
        changed = true;
      }
    }
  }
  if (OEM_BOARD_CODE.test(s)) return "";
  return s;
}

function escapeRe(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Drop one leading series word, longest match first. */
function stripLeadingSeries(s: string): string {
  for (const p of BOARD_SERIES) {
    const re = new RegExp(`^${escapeRe(p)}\\s+`, "i");
    if (re.test(s)) return tidy(s.replace(re, ""));
  }
  return s;
}

/**
 * Drop exactly one tier step from the tail.
 *
 * Order matters. `B550M STEEL LEGEND` has to lose both words at once - a
 * half-stripped `B550M STEEL` is a fragment that reads like a truncation,
 * which is the exact failure this whole module exists to prevent. So the
 * multi-word series groups are tested before the single tier words, and a
 * hyphenated tail keeps its head only when the head is itself a tier word
 * (`B450M PRO-VDH` -> `B450M PRO`, but `B450M-A` -> `B450M`).
 */
function stripOneTail(s: string): string {
  for (const g of BOARD_SERIES) {
    if (!g.includes(" ")) continue;
    const re = new RegExp(`(?:^|\\s)${escapeRe(g)}$`, "i");
    if (re.test(s)) return tidy(s.replace(re, " "));
  }
  const hyphen = s.match(/^(.*)-([A-Z0-9]{1,5})$/);
  if (hyphen) {
    const [, head, tail] = hyphen;
    if (BOARD_TAIL_TIER.includes(tail)) return tidy(head);
  }
  for (const t of BOARD_TAIL_TIER) {
    const re = new RegExp(`(?:^|\\s)${escapeRe(t)}$`, "i");
    if (re.test(s)) return tidy(s.replace(re, " "));
  }
  return s;
}

/** The ladder of board captions, most informative first. */
export function boardCaptionLevels(name: string): string[] {
  const noisy = boardNoise(name);
  if (!noisy) return [];
  const levels = [noisy];
  const serial = stripLeadingSeries(noisy);
  if (serial !== noisy) levels.push(serial);

  let cur = serial;
  // Each pass removes one tier step; a guard on length and on repetition keeps
  // a pathological name (`A B C`) from collapsing to nothing.
  for (let i = 0; i < 6; i++) {
    const next = stripOneTail(cur);
    if (!next || next === cur || next.length < 3) break;
    levels.push(next);
    cur = next;
  }
  return levels;
}

export function shortBoard(name: string): string {
  return pickLevel(boardCaptionLevels(name));
}

// ============================================================== memory

/** Module vendors, by the name firmware writes them. */
const MEM_VENDOR_CN: Record<string, string> = {
  KINGSTON: "金士顿",
  SAMSUNG: "三星",
  HYNIX: "海力士",
  MICRON: "美光",
  CRUCIAL: "英睿达",
  CORSAIR: "海盗船",
  GSKILL: "芝奇",
  ADATA: "威刚",
  TRANSCEND: "创见",
  APACER: "宇瞻",
  PATRIOT: "博帝",
  TEAMGROUP: "十铨",
  TEAM: "十铨",
  PNY: "必恩威",
  KINGBANK: "金百达",
  GLOWAY: "光威",
  NETAC: "朗科",
  RAMAXEL: "记忆科技",
  NANYA: "南亚",
  ELPIDA: "尔必达",
  POWER: "广颖电通",
  ASGARD: "阿斯加特",
  GEIL: "金邦",
  RAMSTA: "瑞士达",
  HIKVISION: "海康",
  LEXAR: "雷克沙",
  CXMT: "长鑫",
  LONGSYS: "江波龙",
  KIMTIGO: "金泰克",
  SPECTEK: "镁光",
  TOSHIBA: "东芝",
  FUJITSU: "富士通",
  WINBOND: "华邦",
  SMART: "世迈",
  INNODISK: "宜鼎",
  AVEXIR: "宇帷",
  MUSHKIN: "魔石",
};

/** Vendor fields matching this are empty, whatever they say. */
const MEM_VENDOR_JUNK =
  /^(?:UNKNOWN|UNDEFINED|NOT SPECIFIED|NOT AVAILABLE|NO DIMM|NONE|N\/A|NA|0+|\[EMPTY\]|<BAD INDEX>|<OUT OF SPEC>|DIMM \d+|ARRAY\d+_MANUFACTURER\d*|MODULEMANUFACTURER)$/i;

/** Part-number fields matching this are empty, whatever they say. */
const MEM_PARTNO_JUNK =
  /^(?:UNKNOWN|UNDEFINED|NOT SPECIFIED|NOT AVAILABLE|NO DIMM|NONE|N\/A|NA|0+|\[EMPTY\]|DIMM \d+|ARRAY\d+_PARTNUMBER\d*|MODULEPARTNUMBER|<NOT PROVIDED>|<BAD INDEX>)$/i;

/**
 * Part-number prefixes that identify a vendor. Firmware leaves the vendor
 * field as `Unknown` often enough to matter - and this is not only a fallback
 * for unreadable part numbers: `HMA851U6CJR6N-VK` beside an empty vendor field
 * resolves to SK Hynix.
 */
const MEM_PARTNO_VENDOR: [RegExp, string][] = [
  [/^(?:99[0-9A-Z]\d{4}|KF\d|KHX|KVR|KSM|KTD|KCP)/, "金士顿"],
  [/^(?:HMA|HMC|HMT|H5A[NGC]|H9H)/, "海力士"],
  [/^(?:M3\d{3}|M4\d{3}|K4[A-Z0-9])/, "三星"],
  [/^(?:MT\d|8ATF|9[0-9A-Z]{4}ATF)/, "美光"],
  [/^(?:CM[K4TRW]|CMV|CM3)/, "海盗船"],
  [/^(?:F4-|F5-|F3-)/, "芝奇"],
  [/^(?:BL\d|CT\d{1,2}G|BLS)/, "英睿达"],
  [/^(?:AD[45]|AX[45]|AM1P)/, "威刚"],
  [/^(?:TLZ|TTC|TED|TEAM|T-CREATE)/, "十铨"],
  [/^(?:PSD|PVS|PSP|PE\d{3})/, "博帝"],
  [/^(?:TS\d{3}|JM\d{3}|T[SVM]\d)/, "创见"],
  [/^(?:GM|GR|GL)/, "光威"],
  [/^(?:KB|KF\d{4,})/, "金百达"],
];

/** A channel part number, not a retail model - nobody can read it. */
function isOemPartNo(pn: string): boolean {
  const t = (pn || "").trim();
  if (!t) return true;
  if (/^[0-9A-Z]{6,10}[-.]\d{2,3}[.,][A-Z]\d{2}[A-Z]?$/.test(t)) return true; // 99P5826-002,A00G
  if (/^\d{7,}[-.]\d{2,3}$/.test(t)) return true;
  if (/^[0-9A-F]{8,}$/i.test(t) && !/[A-Z]{4,}/.test(t)) return true;
  return false;
}

function vendorFromPartNo(pn: string): string {
  const t = (pn || "").trim().toUpperCase();
  for (const [re, v] of MEM_PARTNO_VENDOR) if (re.test(t)) return v;
  return "";
}

function knownVendor(vendor: string): string {
  const t = (vendor || "").trim();
  if (!t || MEM_VENDOR_JUNK.test(t)) return "";
  const up = t.toUpperCase().replace(/[^A-Z0-9]/g, "");
  for (const [k, cn] of Object.entries(MEM_VENDOR_CN)) if (up.includes(k)) return cn;
  return t;
}

/** `DDR4` -> `D4`, `DDR5` -> `D5`, `LPDDR5` -> `L5`. */
function shortDdr(t: string): string {
  return (t || "").replace(/^(LP)?DDR(\d)/i, (_, lp, n) => (lp ? "L" : "D") + n);
}

/**
 * The memory caption.
 *
 * A part number is shown as-is only when it fits. Retail part numbers are
 * perfectly readable *and* 100 px long (`CMWX16GC3200C16W2E`) - keeping them
 * means shipping `CMWX16GC3200C1…`, which is the display problem, not the fix.
 * So the test is "does it fit", not "can it be read", and the fallback is
 * `vendor + type + speed`, which tells a reader more than the number did. The
 * number itself is never lost: it goes on the tooltip.
 */
export function memCaptionLevels(
  partNo: string,
  vendor: string,
  ddr: string,
  mhz: number,
): string[] {
  const raw = (partNo || "").trim();
  const readable = !!raw && !MEM_PARTNO_JUNK.test(raw) && !isOemPartNo(raw);
  if (readable && fitsText(raw)) return [raw];

  const v = knownVendor(vendor) || vendorFromPartNo(raw);
  const spec = [shortDdr(ddr), mhz > 0 ? String(mhz) : ""].filter(Boolean).join(" ");
  const short = tidy([v, spec].filter(Boolean).join(" "));
  if (short) return [short];
  if (readable) return [raw];
  return [];
}

export function shortMem(
  partNo: string,
  vendor: string,
  ddr: string,
  mhz: number,
): string {
  return pickLevel(memCaptionLevels(partNo, vendor, ddr, mhz));
}
