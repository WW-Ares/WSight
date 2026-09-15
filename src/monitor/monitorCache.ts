import type { MonitorCache, Snapshot } from "../shared/types";

/**
 * Rebuild a snapshot-shaped object from the on-disk hardware cache.
 *
 * Everything the cache carries is copied across; everything it does not -
 * because it has to be *measured* rather than remembered - is zeroed. That
 * is the whole point: the panel renders its true layout, with the numbers it
 * already knows, and shows 0 where a number still has to be taken. A cached
 * download speed from an hour ago is not a fact about now, so it is not
 * shown as one.
 *
 * The result is marked as not-live by the caller, which is what keeps the
 * gauges at their full placeholder ring instead of a confident 0%.
 */
export function snapshotFromCache(cache: MonitorCache): Snapshot {
  return {
    ts: cache.savedAtMs,
    uptimeSec: 0,
    cpu: {
      load: 0,
      cores: cache.cpuCores,
      physicalCores: cache.cpuPhysicalCores,
      perCore: [],
      freqMhz: cache.cpuFreqMhz,
      freqLiveMhz: 0,
      brand: cache.cpuBrand,
    },
    mem: {
      total: cache.memTotal,
      used: 0,
      free: 0,
      percent: 0,
      swapTotal: cache.memSwapTotal,
      swapUsed: 0,
      speedMhz: cache.memSpeedMhz,
      ddrType: cache.memDdrType,
      stickCount: cache.memStickCount,
      stickMb: cache.memStickMb,
      vendor: cache.memVendor,
      partNo: cache.memPartNo,
    },
    gpu: cache.gpu
      ? {
          name: cache.gpu.name,
          load: 0,
          memTotal: cache.gpu.memTotal,
          memUsed: 0,
          tempC: null,
          fanPercent: -1,
          powerW: null,
          coreClockMhz: 0,
          memClockMhz: 0,
        }
      : null,
    nets: cache.netName
      ? [
          {
            name: cache.netName,
            rxSec: 0,
            txSec: 0,
            rxTotal: 0,
            txTotal: 0,
            // No address: an IP belongs to one session, and a stale one would
            // read as a fact when it is really a memory.
            ipv4: "",
            linkMbps: cache.netLinkMbps,
          },
        ]
      : [],
    disks: cache.disks.map((d) => ({
      letter: d.letter,
      name: d.name,
      mountPoint: d.mountPoint,
      total: d.total,
      // Used space is a slow-moving quantity, not a rate, so the cached
      // figure is still close enough to be worth showing.
      used: d.used,
      percent: d.percent,
      readSec: 0,
      writeSec: 0,
      isSsd: d.isSsd,
      device: d.device,
    })),
    drives: cache.drives.map((d) => ({
      device: d.device,
      isSsd: d.isSsd,
      letters: d.letters,
      label: d.label,
      readSec: 0,
      writeSec: 0,
    })),
    board: cache.board,
    procCount: 0,
  };
}
