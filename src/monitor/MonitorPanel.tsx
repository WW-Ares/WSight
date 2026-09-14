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
  formatBytesTotal,
  formatClock,
  formatGb,
  formatGbPair,
  formatRateCompact,
  formatUptime,
  rateColor,
} from "../shared/format";
import { Ring } from "./components/Ring";
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

export function MonitorPanel({ config }: { config: AppConfig }) {
  const cfg = useLiveConfig(config);
  const showGpu = cfg.monitorShowGpu;
  const showNet = cfg.monitorShowNet;
  const showDisk = cfg.monitorShowDisk;

  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** observed gap between the two most recent snapshots, for the fps probe */
  const snapGapRef = useRef(0);
  const lastSnapTsRef = useRef(0);

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;

    api
      .getSnapshot()
      .then((s) => {
        if (alive) setSnap(s);
      })
      .catch((e: unknown) => {
        if (alive) setError(String(e));
      });

    onSnapshot((s) => {
      // s.ts is the epoch millis of the sample - the honest interval.
      if (lastSnapTsRef.current) {
        snapGapRef.current = s.ts - lastSnapTsRef.current;
      }
      lastSnapTsRef.current = s.ts;
      if (alive) setSnap(s);
    }).then((u) => {
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
    tiles.length,
    drives.length,
  ]);

  const net = snap?.nets[0] ?? null;

  const cpu = snap?.cpu;
  const cpuTitle = cpu
    ? `CPU ${cpu.load.toFixed(0)}% · ${cpu.cores} 核` +
      (cpu.freqLiveMhz > 0
        ? ` · 实时 ${(cpu.freqLiveMhz / 1000).toFixed(2)}GHz / 额定 ${(cpu.freqMhz / 1000).toFixed(2)}GHz`
        : ` · ${(cpu.freqMhz / 1000).toFixed(2)}GHz`)
    : "CPU";
  // The gauge already shows the average, so the line below it answers the
  // other question: is one core pinned while the rest idle? `高/低` rather
  // than `最高/最低` - the column is only ~80px wide.
  const coreExtremes = cpu?.perCore.length
    ? `高${Math.max(...cpu.perCore).toFixed(0)}% 低${Math.min(...cpu.perCore).toFixed(0)}%`
    : "--";

  const mem = snap?.mem;
  // Swap no longer has a caption line of its own, so the tooltip has to carry
  // it or the figure becomes unreachable.
  const memTitle = mem
    ? `内存 ${mem.percent.toFixed(0)}% · 已用 ${formatGb(mem.used)} / 共 ${formatGb(mem.total)}` +
      ` · 可用 ${formatGb(mem.free)}` +
      ` · 交换 ${formatGb(mem.swapUsed, 0)} / ${formatGb(mem.swapTotal, 0)}` +
      (mem.speedMhz > 0 ? ` · ${mem.speedMhz}MHz` : "")
    : "内存";
  // The second memory line carries the DIMM speed on its own. It used to be
  // `交换0/4G·2133M` squeezed in beside it, but a column is ~75 CSS px wide
  // and both figures together never fitted; swap lives in the tooltip now.
  const memSub = mem && mem.speedMhz > 0 ? `${mem.speedMhz}MHz` : "--";

  const gpu = snap?.gpu;
  const gpuTitle = gpu
    ? `GPU ${gpu.load.toFixed(0)}% · ${gpu.name}` +
      (gpu.tempC !== null ? ` · ${gpu.tempC.toFixed(0)}℃` : "") +
      (gpu.powerW !== null ? ` · ${gpu.powerW.toFixed(0)}W` : "") +
      (gpu.fanPercent >= 0 ? ` · 风扇 ${gpu.fanPercent.toFixed(0)}%` : "") +
      (gpu.memTotal > 0
        ? ` · 显存 ${formatGb(gpu.memUsed)}/${formatGb(gpu.memTotal)}`
        : "")
    : "GPU（无数据）";
  const gpuMem =
    gpu && gpu.memTotal > 0 ? formatGbPair(gpu.memUsed, gpu.memTotal) : "--";
  const gpuThermal = gpu
    ? [
        gpu.tempC !== null && gpu.tempC > 0 ? `${gpu.tempC.toFixed(0)}℃` : null,
        gpu.powerW !== null && gpu.powerW > 0 ? `${gpu.powerW.toFixed(0)}W` : null,
      ]
        .filter(Boolean)
        .join(" · ") || "--"
    : "--";

  return (
    <WidgetFrame
      label="monitor"
      variant="monitor"
      stageRef={stageRef}
      title="WSight"
      sub={snap ? formatUptime(snap.uptimeSec) : "…"}
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
                />
              </div>
              <span className="mon-col-l1">
                {cpu ? formatClock(cpu.freqLiveMhz, cpu.freqMhz) : "--"}
              </span>
              <span className="mon-col-l2">{coreExtremes}</span>
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
                />
              </div>
              <span className="mon-col-l1">
                {mem ? formatGbPair(mem.used, mem.total) : "--"}
              </span>
              <span className="mon-col-l2">{memSub}</span>
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
                  />
                </div>
                <span className="mon-col-l1">{gpuMem}</span>
                <span className="mon-col-l2">{gpuThermal}</span>
              </div>
            ) : null}

            {showNet ? (
              <div className="mon-col">
                {/* The adapter that carries the traffic names the column, so
                    the heading says which link these numbers belong to. */}
                <span className="mon-col-head">
                  {net?.name ?? "--"}
                </span>
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
                {/* `用量` labels the pair underneath: down and up totals now
                    share one line, which only fits as whole units. */}
                <span className="mon-col-l1 net-total-label">用量</span>
                <span
                  className="mon-col-l2"
                  title={
                    `累计下行 ${formatBytes(net?.rxTotal ?? 0)}` +
                    ` · 累计上行 ${formatBytes(net?.txTotal ?? 0)}`
                  }
                >
                  {`↓${formatBytesTotal(net?.rxTotal ?? 0)} ↑${formatBytesTotal(
                    net?.txTotal ?? 0,
                  )}`}
                </span>
              </div>
            ) : null}
          </div>

          {showDisk && (tiles.length || drives.length) ? (
            <>
              <div className="mon-divider" />
              <div
                className="mon-grid mon-disk-block"
                style={{ gridTemplateColumns: diskCols }}
              >
                {tiles.map((disk) => (
                  <DiskTile key={disk.letter} disk={disk} />
                ))}
                {/* The throughput block owns the right-most column - directly
                    under the network gauge. */}
                <div className="disk-io-cell">
                  <DriveIo drives={drives} />
                </div>
              </div>
            </>
          ) : null}
        </>
      )}
    </WidgetFrame>
  );
}
