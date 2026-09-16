import type { Snapshot } from "../shared/types";
import { formatBytesTotal } from "../shared/format";
import { shortBoard, shortCpuModel, shortGpuModel, shortMem } from "./hwName";

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
 *
 * The shortening rules themselves live in `./hwName`, which has no imports so
 * that the corpus test can pull them straight into Node. This file only
 * decides *which* rule applies to a given caption option.
 */

const EMPTY = "--";

export { shortBoard, shortCpuModel, shortGpuModel, shortMem };

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
      return shortBoard(snap.board) || EMPTY;
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
      // The part number only survives when it fits. `CMWX16GC3200C16W2E` is a
      // real, perfectly readable model that is nonetheless 104 px against a
      // 70 px budget - showing it means showing `CMWX16GC3200C1…`. The full
      // string stays reachable on the tooltip.
      return shortMem(mem.partNo, mem.vendor, mem.ddrType, mem.speedMhz) || EMPTY;
    case "board":
      return shortBoard(snap.board) || EMPTY;
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
      return shortBoard(snap.board) || EMPTY;
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
      return shortBoard(snap.board) || EMPTY;
    default:
      return pending || !net
        ? EMPTY
        : `↓${formatBytesTotal(net.rxTotal)} ↑${formatBytesTotal(net.txTotal)}`;
  }
}
