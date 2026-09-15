import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, onSnapshot } from "../shared/api";
import { useLiveConfig } from "../shared/useLiveConfig";
import { useStage } from "../shared/uiScale";
import { WidgetFrame } from "../shared/WidgetFrame";
import type { AppConfig, DiskInfo, DriveInfo, Snapshot } from "../shared/types";
import { MAX_DISK_TILES, MAX_DRIVES } from "../shared/types";
import {
  formatBytes,
  formatClock,
  formatGb,
  formatGbPair,
  formatRateCompact,
  formatUptime,
  rateColor,
} from "../shared/format";
import { Ring } from "./components/Ring";
import { snapshotFromCache } from "./monitorCache";
import {
  secondLineCpu,
  secondLineGpu,
  secondLineMem,
  secondLineNet,
} from "./secondLine";
import "./monitor.css";

/**
 * Volumes the disk block shows: the configured letters, else the first few by
 * letter. A stored selection that no longer matches any present volume falls
 * back to the automatic one instead of leaving the block empty.
 */
function pickVolumes(disks: DiskInfo[], wanted: string[]): DiskInfo[] {
  if (wanted.length) {
    const byLetter = new Map(disks.map((d) => [d.letter, d]));
    const picked = wanted
      .map((letter) => byLetter.get(letter))
      .filter((d): d is DiskInfo => Boolean(d));
    if (picked.length) return picked.slice(0, MAX_DISK_TILES);
  }
  return disks.slice(0, MAX_DISK_TILES);
}

/** Same idea as `pickVolumes`, keyed by physical disk number. */
function pickDrives(drives: DriveInfo[], wanted: number[]): DriveInfo[] {
  if (wanted.length) {
    const picked = wanted
      .map((number) => drives.find((d) => d.device === number))
      .filter((d): d is DriveInfo => Boolean(d));
    if (picked.length) return picked.slice(0, MAX_DRIVES);
  }
  return drives.slice(0, MAX_DRIVES);
}

/** Width of the throughput column, as a multiple of one gauge column. */
const IO_COL_WEIGHT = 1.4;

/** `固态` / `机械` / `磁盘` - what kind of drive this is, when it is known. */
function kindLabel(drive: DriveInfo): string {
  if (drive.isSsd === true) return "固态";
  if (drive.isSsd === false) return "机械";
  return "磁盘";
}

/**
 * One volume: a caption, a narrow fill bar, then used / total.
 *
 * Deliberately just those three things. An SSD/HDD glyph used to sit before
 * the caption, but the block is only ~85px per column and the glyph ate the
 * air the bar needed - the read/write heading beside it already says
 * `固态` / `机械`, so the shape was redundant anyway.
 */
function DiskTile({ disk }: { disk: DiskInfo }) {
  const letter = disk.letter.replace(":", "");
  const used = Number.isFinite(disk.percent) ? disk.percent : 0;
  const width = Math.max(1.5, Math.min(100, used));
  const barColor =
    used >= 92 ? "var(--danger)" : used >= 78 ? "var(--warn)" : "var(--accent-disk)";
  const label = disk.name ? `${letter}: ${disk.name}` : `${letter}:`;

  return (
    <div
      className="disk-tile"
      title={`${label} · 已用 ${formatGb(disk.used)} / ${formatGb(disk.total)}`}
    >
      <span className="disk-name">磁盘 {letter}</span>
      <span className="disk-bar">
        <i style={{ width: `${width}%`, background: barColor }} />
      </span>
      <span className="disk-usage">
        {formatGb(disk.used, 0)} / {formatGb(disk.total, 0)}
      </span>
    </div>
  );
}

/**
 * Read/write throughput per *physical* drive, stacked.
 *
 * Each drive owns two rows - the heading, then read and write side by side -
 * so a drive stays one block instead of two half-columns of numbers. Two
 * drives then come out about as tall as the three volume tiles beside them.
 */
/**
 * `固态 C/D` -> `SSD C+D`: the kind tag plus every volume that lives on that
 * physical drive. Latin tags rather than 固态/机械 because the heading shares
 * one ~110px line with up to three letters, and `+` because these volumes are
 * summed, not listed.
 */
function driveHeading(drive: DriveInfo): string {
  const kind = drive.isSsd === true ? "SSD" : drive.isSsd === false ? "HDD" : "DISK";
  // `letters` arrives as `C:/D:` - the colons are Explorer-speak, and here the
  // volumes are summed rather than listed, so strip them and join with `+`.
  const letters = drive.letters.replace(/:/g, "").replace("/", "+");
  return `${kind} ${letters}`;
}

function DriveIo({ drives }: { drives: DriveInfo[] }) {
  if (!drives.length) return null;
  return (
    <div className="disk-io">
      {drives.map((drive) => (
        <div
          className="disk-io-item"
          key={drive.device}
          title={
            `${kindLabel(drive)} ${drive.letters}` +
            (drive.label ? ` · ${drive.label}` : "") +
            `\n读 ${formatRateCompact(drive.readSec)} · 写 ${formatRateCompact(drive.writeSec)}`
          }
        >
          <span className="disk-io-title">{driveHeading(drive)}</span>
          <span className="disk-io-line">
            <span className="disk-io-op">读</span>
            <span className="disk-io-val">
              {formatRateCompact(drive.readSec, 0)}
            </span>
            <span className="disk-io-op">写</span>
            <span className="disk-io-val">
              {formatRateCompact(drive.writeSec, 0)}
            </span>
          </span>
        </div>
      ))}
    </div>
  );
}

/* -------------------------------------------------------------- placeholders
   Two layers, in the order the panel walks through them.

   First run (no cache on disk yet): the groups below simply own their final
   shape during the beat before the collector's first sample. The gauge row and
   the disk block are independent, and both read the same switches as the real
   thing, so the number of placeholders matches what is about to arrive.

   Every run after that: the panel opens on `monitor-cache.json` - real cores,
   real installed memory, real volumes - with 0 in every field that has to be
   measured. No skeleton is drawn at all, because nothing is missing a shape. */

/**
 * One gauge column: heading chip, empty ring, two caption lines.
 *
 * Each caption holds a non-breaking space rather than a fixed pixel height, so
 * it occupies exactly the line box the real text will - see the note in
 * `monitor.css`.
 */
function GaugeSkeleton() {
  return (
    <div className="mon-col">
      <span className="mon-sk-line mon-sk-head">&nbsp;</span>
      <div className="mon-col-slot">
        <span className="mon-sk-ring" />
      </div>
      <span className="mon-sk-line mon-sk-l1">&nbsp;</span>
      <span className="mon-sk-line mon-sk-l2">&nbsp;</span>
    </div>
  );
}

/** One volume tile: caption, the fill track, then the used/total line. */
function DiskTileSkeleton() {
  return (
    <div className="disk-tile">
      <span className="mon-sk-line mon-sk-disk-name">&nbsp;</span>
      <span className="mon-sk-disk-bar" />
      <span className="mon-sk-line mon-sk-disk-usage">&nbsp;</span>
    </div>
  );
}

/** The throughput column - a single drive, which is the common case. */
function DriveIoSkeleton() {
  return (
    <div className="disk-io">
      <div className="disk-io-item">
        <span className="mon-sk-line mon-sk-io-title">&nbsp;</span>
        <span className="mon-sk-line mon-sk-io-line">&nbsp;</span>
      </div>
    </div>
  );
}

export function MonitorPanel({ config }: { config: AppConfig }) {
  const cfg = useLiveConfig(config);
  const showGpu = cfg.monitorShowGpu;
  const showNet = cfg.monitorShowNet;
  const showDisk = cfg.monitorShowDisk;
  /** Off drops the caption row - and with it the network's "用量" label. */
  const showL2 = cfg.monitorSecondLine;

  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  /**
   * True until the collector's first sample, even when the panel is already
   * drawing remembered hardware. It is what keeps the gauges at their full
   * placeholder ring and the measured figures at 0 - knowing the machine has
   * 64 GB installed is not the same as having measured how much is in use.
   */
  const [pending, setPending] = useState(true);
  /** set once a real sample lands, so a late cache read cannot overwrite it */
  const liveRef = useRef(false);
  /** observed gap between the two most recent snapshots, for the fps probe */
  const snapGapRef = useRef(0);
  const lastSnapTsRef = useRef(0);

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;

    const takeLive = (s: Snapshot) => {
      // s.ts is the epoch millis of the sample - the honest interval.
      if (lastSnapTsRef.current) {
        snapGapRef.current = s.ts - lastSnapTsRef.current;
      }
      lastSnapTsRef.current = s.ts;
      liveRef.current = true;
      if (!alive) return;
      setSnap(s);
      setPending(false);
    };

    api
      .getSnapshot()
      .then((s) => {
        if (s) takeLive(s);
      })
      .catch((e: unknown) => {
        if (alive) setError(String(e));
      });

    // Paint the machine we already know about, before anything is measured.
    // This races `getSnapshot` and the event below by design: the guard drops
    // the cache whenever a live sample got there first.
    api
      .getMonitorCache()
      .then((cached) => {
        if (alive && cached && !liveRef.current) setSnap(snapshotFromCache(cached));
      })
      .catch(() => {
        // No cache is a first run, not a failure - the placeholders cover it.
      });

    onSnapshot(takeLive).then((u) => {
      unlisten = u;
    });

    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  // Diagnostic probe (`debugFps` in config.json): the rendering side can be
  // cleared or convicted with one number - the title carries the real rAF
  // fps plus the observed snapshot interval, so "frame rate vs sample rate"
  // is answerable from the taskbar instead of guessed.
  useEffect(() => {
    if (!cfg.debugFps) return;
    const win = getCurrentWindow();
    let frames = 0;
    let windowStart = performance.now();
    let raf = 0;
    const tick = (now: number) => {
      frames += 1;
      const span = now - windowStart;
      if (span >= 1000) {
        const fps = Math.round((frames * 1000) / span);
        frames = 0;
        windowStart = now;
        void win.setTitle(
          `WSight Monitor | ${fps}fps · snap ${Math.round(snapGapRef.current)}ms`,
        );
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [cfg.debugFps]);

  // Column count drives both the gauge row and the disk block, so the tiles
  // always sit directly under the gauges.
  const cols = 2 + (showGpu ? 1 : 0) + (showNet ? 1 : 0);

  // One sample spread over ~90% of the sampling interval: the eased gauges
  // are then in motion almost the whole time between snapshots, which is
  // what makes the rings read as continuous instead of once-a-second ticks.
  const ringRamp = Math.max(200, Math.round(cfg.monitorIntervalMs * 0.9));

  const volumes = useMemo(
    () => (snap ? pickVolumes(snap.disks, cfg.monitorDisks) : []),
    [snap, cfg.monitorDisks],
  );
  const drives = useMemo(
    () => (snap ? pickDrives(snap.drives, cfg.monitorDrives) : []),
    [snap, cfg.monitorDrives],
  );
  // The throughput block owns the right-most slot, so the tiles get the rest.
  const tileSlots = Math.max(0, cols - 1);
  const tiles = volumes.slice(0, tileSlots);
  // That slot is weighted a little heavier than a gauge's share: two drive
  // labels plus two rates have to fit side by side in it, while the tiles
  // beside it are only a 76px bar wide.
  const diskCols =
    `repeat(${Math.max(1, cols - 1)}, minmax(0, 1fr))` +
    ` minmax(0, ${IO_COL_WEIGHT}fr)`;

  const stageRef = useStage("monitor", [
    showGpu,
    showNet,
    showDisk,
    showL2,
    tiles.length,
    drives.length,
  ]);

  const net = snap?.nets[0] ?? null;

  const cpu = snap?.cpu;
  // While pending, a tooltip may only say what is known for certain - the
  // hardware. A "0%" in there would be read as a measurement when it is really
  // the absence of one.
  const cpuTitle = !cpu
    ? "CPU"
    : pending
      ? `CPU ${cpu.cores} 核 · ${(cpu.freqMhz / 1000).toFixed(2)}GHz · 等待首次采样`
      : `CPU ${cpu.load.toFixed(0)}% · ${cpu.cores} 核` +
        (cpu.freqLiveMhz > 0
          ? ` · 实时 ${(cpu.freqLiveMhz / 1000).toFixed(2)}GHz / 额定 ${(cpu.freqMhz / 1000).toFixed(2)}GHz`
          : ` · ${(cpu.freqMhz / 1000).toFixed(2)}GHz`);
  const mem = snap?.mem;
  // Swap no longer has a caption line of its own, so the tooltip has to carry
  // it or the figure becomes unreachable.
  const memTitle = !mem
    ? "内存"
    : pending
      ? `内存 共 ${formatGb(mem.total)}` +
        (mem.speedMhz > 0 ? ` · ${mem.speedMhz}MHz` : "") +
        " · 等待首次采样"
      : `内存 ${mem.percent.toFixed(0)}% · 已用 ${formatGb(mem.used)} / 共 ${formatGb(mem.total)}` +
        ` · 可用 ${formatGb(mem.free)}` +
        ` · 交换 ${formatGb(mem.swapUsed, 0)} / ${formatGb(mem.swapTotal, 0)}` +
        (mem.speedMhz > 0 ? ` · ${mem.speedMhz}MHz` : "");
  const gpu = snap?.gpu;
  const gpuTitle = !gpu
    ? "GPU（无数据）"
    : pending
      ? `GPU ${gpu.name} · 等待首次采样`
      : `GPU ${gpu.load.toFixed(0)}% · ${gpu.name}` +
        (gpu.tempC !== null ? ` · ${gpu.tempC.toFixed(0)}℃` : "") +
        (gpu.powerW !== null ? ` · ${gpu.powerW.toFixed(0)}W` : "") +
        (gpu.fanPercent >= 0 ? ` · 风扇 ${gpu.fanPercent.toFixed(0)}%` : "") +
        (gpu.memTotal > 0
          ? ` · 显存 ${formatGb(gpu.memUsed)}/${formatGb(gpu.memTotal)}`
          : "");
  const gpuMem =
    gpu && gpu.memTotal > 0 ? formatGbPair(gpu.memUsed, gpu.memTotal) : "--";

  // The caption of each column, and - for the network - the label above it.
  // The other three columns put a measurement on their first line and let the
  // caption speak for itself; the network has no equivalent figure there, so
  // its first line names what the second one says.
  const l2Cpu = secondLineCpu(snap, pending, cfg.monitorL2Cpu);
  const l2Mem = secondLineMem(snap, cfg.monitorL2Mem);
  const l2Gpu = secondLineGpu(snap, pending, cfg.monitorL2Gpu);
  const l2Net = secondLineNet(snap, pending, cfg.monitorL2Net);
  const netLabel =
    cfg.monitorL2Net === "ip"
      ? "IP"
      : cfg.monitorL2Net === "link"
        ? "链路"
        : cfg.monitorL2Net === "board"
          ? "主板"
          : "用量";

  // Two flags, not one: `ready` says the panel has a shape to draw - from the
  // cache or from a sample - and `pending` says the numbers in it are still
  // the remembered ones. Once the first sample lands, both flip together.
  const ready = snap !== null;
  const showDiskBlock =
    showDisk && (!ready || tiles.length > 0 || drives.length > 0);

  return (
    <WidgetFrame
      label="monitor"
      variant="monitor"
      stageRef={stageRef}
      title="WSight"
      sub={!pending && snap ? formatUptime(snap.uptimeSec) : "启动中…"}
      onTop={cfg.monitorAlwaysOnTop}
    >
      {error && !snap ? (
        <div className="monitor-error">采集失败：{error}</div>
      ) : (
        <>
          <div
            className="mon-grid"
            style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}
          >
            {ready ? (
              <>
                <div className="mon-col">
                  <span className="mon-col-head" style={{ color: "var(--accent-cpu)" }}>
                    CPU
                  </span>
                  <div className="mon-col-slot">
                    <Ring
                      value={cpu?.load ?? 0}
                      color="var(--accent-cpu)"
                      ramp={ringRamp}
                      title={cpuTitle}
                      pending={pending}
                    />
                  </div>
                  <span className="mon-col-l1">
                    {cpu ? formatClock(cpu.freqLiveMhz, cpu.freqMhz) : "--"}
                  </span>
                  {showL2 ? <span className="mon-col-l2">{l2Cpu}</span> : null}
                </div>

                <div className="mon-col">
                  <span className="mon-col-head" style={{ color: "var(--accent-mem)" }}>
                    Mem
                  </span>
                  <div className="mon-col-slot">
                    <Ring
                      value={mem?.percent ?? 0}
                      color="var(--accent-mem)"
                      ramp={ringRamp}
                      title={memTitle}
                      pending={pending}
                    />
                  </div>
                  <span className="mon-col-l1">
                    {mem ? formatGbPair(mem.used, mem.total) : "--"}
                  </span>
                  {showL2 ? <span className="mon-col-l2">{l2Mem}</span> : null}
                </div>

                {showGpu ? (
                  <div className="mon-col">
                    <span className="mon-col-head" style={{ color: "var(--accent-gpu)" }}>
                      GPU
                    </span>
                    <div className="mon-col-slot">
                    <Ring
                      value={gpu?.load ?? 0}
                      color="var(--accent-gpu)"
                      ramp={ringRamp}
                      title={gpuTitle}
                      pending={pending}
                    />
                    </div>
                    <span className="mon-col-l1">{gpuMem}</span>
                    {showL2 ? <span className="mon-col-l2">{l2Gpu}</span> : null}
                  </div>
                ) : null}

                {showNet ? (
                  <div className="mon-col">
                    {/* The adapter that carries the traffic names the column,
                        so the heading says which link these numbers belong to. */}
                    <span className="mon-col-head">{net?.name ?? "--"}</span>
                    <div className="mon-col-slot">
                      <div className="net-pair">
                        <span className="net-line">
                          <i className="net-arrow down">↓</i>
                          <b style={{ color: rateColor(net?.rxSec ?? 0, cfg.colorNet) }}>
                            {formatRateCompact(net?.rxSec ?? 0)}
                          </b>
                        </span>
                        <span className="net-line">
                          <i className="net-arrow up">↑</i>
                          <b style={{ color: rateColor(net?.txSec ?? 0, "#ffd479") }}>
                            {formatRateCompact(net?.txSec ?? 0)}
                          </b>
                        </span>
                      </div>
                    </div>
                    {/* The label names what the caption below it says - the
                        totals, an address, the link speed - since this column
                        has no measured figure of its own to put here. */}
                    {showL2 ? (
                      <>
                        <span className="mon-col-l1 net-total-label">
                          {netLabel}
                        </span>
                        <span
                          className="mon-col-l2"
                          title={
                            `累计下行 ${formatBytes(net?.rxTotal ?? 0)}` +
                            ` · 累计上行 ${formatBytes(net?.txTotal ?? 0)}`
                          }
                        >
                          {l2Net}
                        </span>
                      </>
                    ) : null}
                  </div>
                ) : null}
              </>
            ) : (
              // One placeholder per column, so switching GPU or the network
              // gauge off narrows the skeleton the same way it narrows the row.
              Array.from({ length: cols }, (_, i) => <GaugeSkeleton key={i} />)
            )}
          </div>

          {showDiskBlock ? (
            <>
              <div className="mon-divider" />
              <div
                className="mon-grid mon-disk-block"
                style={{ gridTemplateColumns: diskCols }}
              >
                {ready ? (
                  <>
                    {tiles.map((disk) => (
                      <DiskTile key={disk.letter} disk={disk} />
                    ))}
                    {/* The throughput block owns the right-most column -
                        directly under the network gauge. */}
                    <div className="disk-io-cell">
                      <DriveIo drives={drives} />
                    </div>
                  </>
                ) : (
                  <>
                    {Array.from({ length: tileSlots }, (_, i) => (
                      <DiskTileSkeleton key={i} />
                    ))}
                    <div className="disk-io-cell">
                      <DriveIoSkeleton />
                    </div>
                  </>
                )}
              </div>
            </>
          ) : null}
        </>
      )}
    </WidgetFrame>
  );
}
