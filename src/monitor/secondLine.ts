import type { Snapshot } from "../shared/types";
import { formatBytesTotal } from "../shared/format";

/**
 * The caption line under each gauge, in one place.
 *
 * The panel and the settings window both need these strings - the panel to
 * draw them, the settings window to describe what each option means - so the
 * rules live here instead of being written twice.
 *
 * Two kinds of value appear: facts about the hardware, which are known the
 * moment the app starts (model, DDR type, board), and measurements, which are
 * not. A measurement taken before the collector's first sample is shown as
 * `--` rather than 0: 0% is a reading, `--` is an admission.
 */

const EMPTY = "--";

/** `Intel(R) Core(TM) i9-10900K CPU @ 3.70GHz` -> `i9-10900K`. */
export function shortCpuModel(brand: string): string {
  if (!brand) return "";
  let s = brand.replace(/\((?:R|TM|C)\)/gi, " ");
  // The trailing "@ 3.70GHz" is the rated clock, which the line above the
  // caption already shows live.
  s = s.replace(/@.*$/, " ");
  s = s.replace(/\b(?:Intel|AMD)\b/gi, " ");
  s = s.replace(/\b(?:CPU|Processor|APU)\b/gi, " ");
  // "Core" only survives when it is not the filler in front of the model:
  // `Core i9-10900K` and `Core Ultra 7 155H` both read better without it,
  // while `Core 2 Duo` is nothing but that word.
  s = s.replace(/\bCore\s+(?=i\d|Ultra)/gi, "");
  s = s.replace(/\b\d+\s*[-–]?\s*Core\b/gi, "");
  s = s.replace(/with Radeon(?: Graphics)?/gi, " ");
  return s.replace(/\s+/g, " ").trim();
}

/** `NVIDIA GeForce RTX 2080 Ti` -> `RTX 2080 Ti`. */
export function shortGpuModel(name: string): string {
  if (!name) return "";
  let s = name.replace(/\b(?:NVIDIA|AMD|Intel)\b/gi, " ");
  s = s.replace(/\b(?:GeForce|Radeon|Arc)\b/gi, " ");
  return s.replace(/\s+/g, " ").trim();
}

/** 1000 -> `1.0G`, 100 -> `100M`. Gbit links are the common case now. */
function linkText(mbps: number): string {
  if (!mbps) return EMPTY;
  return mbps >= 1000 ? `${(mbps / 1000).toFixed(1)}G` : `${mbps}M`;
}

function join(parts: (string | null)[]): string {
  return parts.filter(Boolean).join(" · ") || EMPTY;
}

export function secondLineCpu(
  snap: Snapshot | null,
  pending: boolean,
  mode: string,
): string {
  if (!snap) return EMPTY;
  const cpu = snap.cpu;
  switch (mode) {
    case "model":
      return shortCpuModel(cpu.brand) || EMPTY;
    case "cores":
      return cpu.physicalCores > 0
        ? `${cpu.physicalCores}C ${cpu.cores}T`
        : `${cpu.cores}T`;
    case "procs":
      return pending ? EMPTY : `${snap.procCount} 进程`;
    case "board":
      return snap.board || EMPTY;
    default: {
      // The gauge already shows the average; this answers the other question
      // - is one core pinned while the rest idle?
      if (pending || !cpu.perCore.length) return EMPTY;
      const high = Math.max(...cpu.perCore).toFixed(0);
      const low = Math.min(...cpu.perCore).toFixed(0);
      return `高${high}% 低${low}%`;
    }
  }
}

export function secondLineMem(snap: Snapshot | null, mode: string): string {
  if (!snap) return EMPTY;
  const mem = snap.mem;
  switch (mode) {
    case "ddr":
      return join([
        mem.ddrType || null,
        mem.speedMhz > 0 ? `${mem.speedMhz}` : null,
      ]);
    case "sticks":
      return mem.stickCount > 0
        ? `${mem.stickCount}×${Math.round(mem.stickMb / 1024)}G`
        : EMPTY;
    case "partno":
      return mem.partNo || EMPTY;
    case "board":
      return snap.board || EMPTY;
    default:
      return mem.speedMhz > 0 ? `${mem.speedMhz}MHz` : EMPTY;
  }
}

export function secondLineGpu(
  snap: Snapshot | null,
  pending: boolean,
  mode: string,
): string {
  const gpu = snap?.gpu;
  if (!snap || !gpu) return EMPTY;
  switch (mode) {
    case "model":
      return shortGpuModel(gpu.name) || EMPTY;
    case "clocks":
      return pending || !gpu.coreClockMhz
        ? EMPTY
        : `${gpu.coreClockMhz}/${gpu.memClockMhz}`;
    case "fan":
      return pending
        ? EMPTY
        : join([
            gpu.tempC !== null && gpu.tempC > 0 ? `${gpu.tempC.toFixed(0)}℃` : null,
            gpu.fanPercent >= 0 ? `${gpu.fanPercent.toFixed(0)}%` : null,
          ]);
    case "board":
      return snap.board || EMPTY;
    default:
      return pending
        ? EMPTY
        : join([
            gpu.tempC !== null && gpu.tempC > 0 ? `${gpu.tempC.toFixed(0)}℃` : null,
            gpu.powerW !== null && gpu.powerW > 0 ? `${gpu.powerW.toFixed(0)}W` : null,
          ]);
  }
}

export function secondLineNet(
  snap: Snapshot | null,
  pending: boolean,
  mode: string,
): string {
  if (!snap) return EMPTY;
  const net = snap.nets[0];
  switch (mode) {
    case "ip":
      return net?.ipv4 || EMPTY;
    case "link":
      return net ? linkText(net.linkMbps) : EMPTY;
    case "board":
      return snap.board || EMPTY;
    default:
      return pending || !net
        ? EMPTY
        : `↓${formatBytesTotal(net.rxTotal)} ↑${formatBytesTotal(net.txTotal)}`;
  }
}
