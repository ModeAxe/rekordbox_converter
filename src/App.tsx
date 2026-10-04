import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";

type FfmpegStatus =
  | { state: "checking" }
  | { state: "ok"; version: string }
  | { state: "error"; message: string };

type DriveInfo = {
  root: string;
  label: string;
  driveType: string;
  isRekordboxExport: boolean;
};

type ExportScan = {
  root: string;
  exportPdb: string;
  exportExtPdb: string | null;
  hasExportExt: boolean;
  totalTracks: number;
  convertibleCount: number;
  mp3Count: number;
  otherCount: number;
  convertibleBytes: number;
  estimatedSeconds: number;
  convertiblePaths: string[];
};

type ScanState =
  | { state: "idle" }
  | { state: "scanning" }
  | { state: "ready"; scan: ExportScan; estimate: string }
  | { state: "error"; message: string };

type RoundtripLibraryResult = {
  parsed: boolean;
  byteIdentical: boolean;
  trackCount: number | null;
  message: string;
};

type ExportInspect = {
  root: string;
  summary: {
    trackCount: number;
    playlistCount: number;
    folderCount: number;
    flacInDb: number;
    mp3InDb: number;
    otherInDb: number;
    anlzFiles: number;
    anlzPpathMismatches: number;
    hasExportExt: boolean;
    myTagCount: number;
  };
  roundtrip: {
    rekordboxPdb: RoundtripLibraryResult;
    rekordcrate: RoundtripLibraryResult;
    recommended: string;
  };
  playlists: {
    id: number;
    name: string;
    parentId: number;
    sortOrder: number;
    isFolder: boolean;
    trackCount: number;
  }[];
  textDump: string;
};

type InspectState =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ready"; data: ExportInspect }
  | { state: "error"; message: string };

type ConvertProgress = {
  completed: number;
  total: number;
  currentFile: string;
  outcome: "cacheHit" | "converted" | "failed";
  cachePath: string | null;
  error: string | null;
  cacheHits: number;
  converted: number;
  failed: number;
};

type ConvertSummary = {
  total: number;
  cacheHits: number;
  converted: number;
  failed: number;
  cacheRoot: string;
  errors: string[];
};

type ConvertState =
  | { state: "idle" }
  | { state: "running"; progress: ConvertProgress | null }
  | { state: "done"; summary: ConvertSummary }
  | { state: "error"; message: string };

type PipelineProgress = {
  phase: string;
  completed: number;
  total: number;
  detail: string;
};

type StagedCopySummary = {
  outputRoot: string;
  tracksConverted: number;
  cacheHits: number;
  pdbUpdated: number;
  anlzUpdated: number;
  sourcesRemoved: number;
  verifyProblems: string[];
  errors: string[];
};

type PipelineState =
  | { state: "idle" }
  | { state: "running"; progress: PipelineProgress | null }
  | { state: "done"; summary: StagedCopySummary }
  | { state: "error"; message: string };

type InPlaceSummary = {
  usbRoot: string;
  tracksConverted: number;
  cacheHits: number;
  pdbUpdated: number;
  anlzUpdated: number;
  sourcesRemoved: number;
  rolledBack: boolean;
  backupKept: boolean;
  verifyProblems: string[];
  errors: string[];
};

type InPlaceState =
  | { state: "idle" }
  | { state: "running"; progress: PipelineProgress | null }
  | { state: "done"; summary: InPlaceSummary }
  | { state: "error"; message: string };

type PlaylistOption = {
  id: number;
  name: string;
  depth: number;
  trackCount: number;
  convertibleCount: number;
  mp3Count: number;
  otherCount: number;
};

type ConvertScope = {
  playlistId: number | null;
  playlistName: string | null;
  convertiblePaths: string[];
  convertibleCount: number;
  estimatedSeconds: number;
};

type AppSettings = {
  bitrate: string;
  cacheRoot: string;
  workers: number;
  keepSource: boolean;
  verifyOutput: boolean;
  dryRun: boolean;
};

type TabId = "convert" | "inspect" | "settings";

const PLAYLIST_ALL = "";
const POLL_MS = 2000;
const DEFAULT_SETTINGS: AppSettings = {
  bitrate: "320k",
  cacheRoot: "",
  workers: 0,
  keepSource: false,
  verifyOutput: true,
  dryRun: false,
};

export default function App() {
  const [appVersion, setAppVersion] = useState("");
  const [tab, setTab] = useState<TabId>("convert");
  const [ffmpeg, setFfmpeg] = useState<FfmpegStatus>({ state: "checking" });
  const [drives, setDrives] = useState<DriveInfo[]>([]);
  const [selectedRoot, setSelectedRoot] = useState("");
  const [scanState, setScanState] = useState<ScanState>({ state: "idle" });
  const [inspectState, setInspectState] = useState<InspectState>({
    state: "idle",
  });
  const [convertState, setConvertState] = useState<ConvertState>({
    state: "idle",
  });
  const [pipelineState, setPipelineState] = useState<PipelineState>({
    state: "idle",
  });
  const [inplaceState, setInplaceState] = useState<InPlaceState>({
    state: "idle",
  });
  const [defaultCacheRoot, setDefaultCacheRoot] = useState("");
  const [stagedParent, setStagedParent] = useState("");
  const [playlists, setPlaylists] = useState<PlaylistOption[]>([]);
  const [playlistSelect, setPlaylistSelect] = useState(PLAYLIST_ALL);
  const [scope, setScope] = useState<ConvertScope | null>(null);
  const [scopeEstimate, setScopeEstimate] = useState("—");
  const [scopeLoading, setScopeLoading] = useState(false);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [settingsDraft, setSettingsDraft] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [settingsMsg, setSettingsMsg] = useState("");
  const [confirmInPlace, setConfirmInPlace] = useState<{
    scopeLabel: string;
    root: string;
  } | null>(null);
  const confirmDefaultRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => setAppVersion("?"));
    invoke<string>("ffmpeg_version")
      .then((version) => setFfmpeg({ state: "ok", version }))
      .catch((err) => setFfmpeg({ state: "error", message: String(err) }));
    invoke<{ root: string }>("get_cache_root")
      .then((info) => setDefaultCacheRoot(info.root))
      .catch(() => setDefaultCacheRoot(""));
    invoke<{ root: string }>("get_staged_parent")
      .then((info) => setStagedParent(info.root))
      .catch(() => setStagedParent(""));
    invoke<AppSettings>("get_settings")
      .then((s) => {
        setSettings(s);
        setSettingsDraft(s);
      })
      .catch(() => {
        /* defaults */
      });
  }, []);

  useEffect(() => {
    let cancelled = false;
    const refresh = async () => {
      try {
        const next = await invoke<DriveInfo[]>("list_drives");
        if (cancelled) return;
        setDrives(next);
        setSelectedRoot((prev) => {
          if (prev && next.some((d) => d.root === prev)) return prev;
          const preferred =
            next.find((d) => d.isRekordboxExport) ?? next[0] ?? null;
          return preferred?.root ?? "";
        });
      } catch {
        /* keep previous */
      }
    };
    refresh();
    const id = window.setInterval(refresh, POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  useEffect(() => {
    if (!selectedRoot) {
      setScanState({ state: "idle" });
      setPlaylists([]);
      return;
    }
    let cancelled = false;
    (async () => {
      setScanState({ state: "scanning" });
      try {
        const scan = await invoke<ExportScan>("scan_drive", {
          root: selectedRoot,
        });
        const estimate = await invoke<string>("format_estimate", {
          seconds: scan.estimatedSeconds,
        });
        if (cancelled) return;
        setScanState({ state: "ready", scan, estimate });
        if (scan.convertibleCount > 0 || scan.totalTracks > 0) {
          const list = await invoke<PlaylistOption[]>("list_export_playlists", {
            root: selectedRoot,
          });
          if (!cancelled) setPlaylists(list);
        } else {
          setPlaylists([]);
        }
      } catch (err) {
        if (!cancelled) setScanState({ state: "error", message: String(err) });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [selectedRoot]);

  useEffect(() => {
    if (scanState.state !== "ready" || !selectedRoot) {
      setScope(null);
      setScopeEstimate("—");
      return;
    }
    let cancelled = false;
    setScopeLoading(true);
    const playlistId =
      playlistSelect === PLAYLIST_ALL ? null : Number(playlistSelect);
    (async () => {
      try {
        const next = await invoke<ConvertScope>("get_convert_scope", {
          root: selectedRoot,
          playlistId,
        });
        const estimate = await invoke<string>("format_estimate", {
          seconds: next.estimatedSeconds,
        });
        if (cancelled) return;
        setScope(next);
        setScopeEstimate(estimate);
      } catch (err) {
        console.error(err);
      } finally {
        if (!cancelled) setScopeLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [selectedRoot, playlistSelect, scanState]);

  useEffect(() => {
    let u1: (() => void) | undefined;
    let u2: (() => void) | undefined;
    let u3: (() => void) | undefined;
    let u4: (() => void) | undefined;
    let u5: (() => void) | undefined;
    let u6: (() => void) | undefined;

    listen<ConvertProgress>("conversion-progress", (e) => {
      setConvertState({ state: "running", progress: e.payload });
    }).then((fn) => {
      u1 = fn;
    });
    listen<ConvertSummary>("conversion-finished", (e) => {
      setConvertState({ state: "done", summary: e.payload });
    }).then((fn) => {
      u2 = fn;
    });
    listen<PipelineProgress>("pipeline-progress", (e) => {
      setPipelineState({ state: "running", progress: e.payload });
    }).then((fn) => {
      u3 = fn;
    });
    listen<StagedCopySummary>("pipeline-finished", (e) => {
      setPipelineState({ state: "done", summary: e.payload });
    }).then((fn) => {
      u4 = fn;
    });
    listen<PipelineProgress>("inplace-progress", (e) => {
      setInplaceState({ state: "running", progress: e.payload });
    }).then((fn) => {
      u5 = fn;
    });
    listen<InPlaceSummary>("inplace-finished", (e) => {
      setInplaceState({ state: "done", summary: e.payload });
    }).then((fn) => {
      u6 = fn;
    });

    return () => {
      u1?.();
      u2?.();
      u3?.();
      u4?.();
      u5?.();
      u6?.();
    };
  }, []);

  useEffect(() => {
    if (!confirmInPlace) return;
    confirmDefaultRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        setConfirmInPlace(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [confirmInPlace]);

  const selected = drives.find((d) => d.root === selectedRoot);
  const converting = convertState.state === "running";
  const staging = pipelineState.state === "running";
  const inplace = inplaceState.state === "running";
  const busy = converting || staging || inplace;
  const scopeConvertibleCount = scope?.convertibleCount ?? 0;
  const canConvert =
    scanState.state === "ready" &&
    scopeConvertibleCount > 0 &&
    ffmpeg.state === "ok" &&
    !busy &&
    !scopeLoading;
  const canInspect = (selected?.isRekordboxExport ?? false) && !busy;

  const statusLine = (() => {
    if (converting && convertState.progress) {
      return `${convertState.progress.completed}/${convertState.progress.total} ${convertState.progress.currentFile}`;
    }
    if (staging && pipelineState.progress) {
      return `${pipelineState.progress.phase}: ${pipelineState.progress.detail}`;
    }
    if (inplace && inplaceState.progress) {
      return `${inplaceState.progress.phase}: ${inplaceState.progress.detail}`;
    }
    if (busy) return "Working…";
    if (ffmpeg.state === "error") return "FFmpeg missing";
    if (!selectedRoot) return "Insert a USB drive";
    if (selected && !selected.isRekordboxExport) return "No export.pdb on drive";
    if (scanState.state === "ready") {
      return `${scanState.scan.convertibleCount} to convert · ${scanState.scan.mp3Count} MP3 · est ${scopeEstimate}`;
    }
    if (scanState.state === "scanning") return "Scanning…";
    return "Ready";
  })();

  const resultText = (() => {
    if (inplaceState.state === "done" && inplaceState.summary.rolledBack) {
      return `Rolled back. ${inplaceState.summary.errors.join(" ")}`;
    }
    if (inplaceState.state === "done") {
      const s = inplaceState.summary;
      if (s.errors.some((e) => e.startsWith("dry run"))) {
        return s.errors[0];
      }
      return `USB OK — ${s.pdbUpdated} tracks, ${s.sourcesRemoved} sources removed`;
    }
    if (pipelineState.state === "done") {
      const s = pipelineState.summary;
      return `Copy ready — ${s.outputRoot}`;
    }
    if (convertState.state === "done") {
      const s = convertState.summary;
      return `Cache: ${s.converted} new, ${s.cacheHits} hits`;
    }
    if (inplaceState.state === "error") return inplaceState.message;
    if (pipelineState.state === "error") return pipelineState.message;
    if (convertState.state === "error") return convertState.message;
    return "";
  })();

  const startConvert = async () => {
    if (!selectedRoot) return;
    setConvertState({ state: "running", progress: null });
    const playlistId =
      playlistSelect === PLAYLIST_ALL ? null : Number(playlistSelect);
    try {
      const summary = await invoke<ConvertSummary>("convert_to_cache", {
        root: selectedRoot,
        playlistId,
      });
      setConvertState({ state: "done", summary });
    } catch (err) {
      setConvertState({ state: "error", message: String(err) });
    }
  };

  const startStagedCopy = async () => {
    if (!selectedRoot) return;
    setPipelineState({ state: "running", progress: null });
    const playlistId =
      playlistSelect === PLAYLIST_ALL ? null : Number(playlistSelect);
    try {
      const summary = await invoke<StagedCopySummary>("build_staged_copy", {
        root: selectedRoot,
        playlistId,
        outputName: null,
      });
      setPipelineState({ state: "done", summary });
    } catch (err) {
      setPipelineState({ state: "error", message: String(err) });
    }
  };

  const runInPlace = async () => {
    if (!selectedRoot) return;
    setConfirmInPlace(null);
    setInplaceState({ state: "running", progress: null });
    const playlistId =
      playlistSelect === PLAYLIST_ALL ? null : Number(playlistSelect);
    try {
      const summary = await invoke<InPlaceSummary>("convert_usb_inplace", {
        root: selectedRoot,
        playlistId,
      });
      setInplaceState({ state: "done", summary });
      if (!summary.rolledBack && !settings.dryRun) {
        try {
          const scan = await invoke<ExportScan>("scan_drive", {
            root: selectedRoot,
          });
          const estimate = await invoke<string>("format_estimate", {
            seconds: scan.estimatedSeconds,
          });
          setScanState({ state: "ready", scan, estimate });
        } catch {
          /* keep */
        }
      }
    } catch (err) {
      setInplaceState({ state: "error", message: String(err) });
    }
  };

  const startInPlace = () => {
    if (!selectedRoot) return;
    if (!settings.dryRun) {
      const scopeLabel =
        playlistSelect === PLAYLIST_ALL
          ? "ALL non-MP3 audio on this USB"
          : `playlist (${scope?.playlistName ?? "selected"})`;
      setConfirmInPlace({ scopeLabel, root: selectedRoot });
      return;
    }
    void runInPlace();
  };

  const loadInspect = async () => {
    if (!selectedRoot) return;
    setTab("inspect");
    setInspectState({ state: "loading" });
    try {
      const data = await invoke<ExportInspect>("inspect_drive", {
        root: selectedRoot,
      });
      setInspectState({ state: "ready", data });
    } catch (err) {
      setInspectState({ state: "error", message: String(err) });
    }
  };

  const saveSettings = async () => {
    try {
      const saved = await invoke<AppSettings>("save_settings", {
        settings: settingsDraft,
      });
      setSettings(saved);
      setSettingsDraft(saved);
      setSettingsMsg("Settings saved.");
      const info = await invoke<{ root: string }>("get_cache_root");
      setDefaultCacheRoot(info.root);
    } catch (err) {
      setSettingsMsg(String(err));
    }
  };

  const progressPct =
    convertState.state === "running" && convertState.progress
      ? Math.round(
          (convertState.progress.completed /
            Math.max(1, convertState.progress.total)) *
            100,
        )
      : staging && pipelineState.progress && pipelineState.progress.total > 0
        ? Math.round(
            (pipelineState.progress.completed / pipelineState.progress.total) *
              100,
          )
        : inplace && inplaceState.progress && inplaceState.progress.total > 0
          ? Math.round(
              (inplaceState.progress.completed / inplaceState.progress.total) *
                100,
            )
          : busy
            ? 10
            : 0;

  return (
    <div className="shell">
      <menu role="tablist">
        {(
          [
            ["convert", "Convert"],
            ["inspect", "Inspect"],
            ["settings", "Settings"],
          ] as const
        ).map(([id, label]) => (
          <li
            key={id}
            role="tab"
            aria-selected={tab === id}
            onClick={() => setTab(id)}
          >
            <a
              href={`#${id}`}
              onClick={(e) => {
                e.preventDefault();
                setTab(id);
              }}
            >
              {label}
            </a>
          </li>
        ))}
      </menu>

      <div className="window tab-panel" role="tabpanel">
        <div className="window-body">
          {tab === "convert" && (
            <>
              <fieldset>
                <legend>USB</legend>
                <div className="field-row stacked-control">
                  <label htmlFor="drive">Drive</label>
                  <select
                    id="drive"
                    value={selectedRoot}
                    disabled={busy || drives.length === 0}
                    onChange={(e) => setSelectedRoot(e.target.value)}
                  >
                    {drives.length === 0 && (
                      <option value="">(no removable drives)</option>
                    )}
                    {drives.map((d) => (
                      <option key={d.root} value={d.root}>
                        {d.label}
                        {d.isRekordboxExport ? " *" : ""}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="field-row">
                  <label>FFmpeg</label>
                  <span className="value">
                    {ffmpeg.state === "ok"
                      ? "Ready"
                      : ffmpeg.state === "checking"
                        ? "…"
                        : "Missing"}
                  </span>
                </div>
                <div className="field-row">
                  <label>Export</label>
                  <span className="value">
                    {scanState.state === "ready"
                      ? `${scanState.scan.totalTracks} tracks · ${scanState.scan.convertibleCount} to convert · ${scanState.scan.mp3Count} MP3`
                      : scanState.state === "scanning"
                        ? "Scanning…"
                        : scanState.state === "error"
                          ? scanState.message
                          : "—"}
                  </span>
                </div>
              </fieldset>

              <fieldset>
                <legend>Convert</legend>
                <div className="field-row stacked-control">
                  <label htmlFor="playlist">Scope</label>
                  <select
                    id="playlist"
                    value={playlistSelect}
                    disabled={busy || scanState.state !== "ready"}
                    onChange={(e) => setPlaylistSelect(e.target.value)}
                  >
                    <option value={PLAYLIST_ALL}>
                      All non-MP3
                      {scanState.state === "ready"
                        ? ` (${scanState.scan.convertibleCount})`
                        : ""}
                    </option>
                    {playlists.map((p) => (
                      <option key={p.id} value={String(p.id)}>
                        {"\u00A0".repeat(p.depth * 2)}
                        {p.name} ({p.convertibleCount} convert)
                      </option>
                    ))}
                  </select>
                </div>
                <div className="field-row">
                  <label>Estimate</label>
                  <span className="value">
                    {scopeLoading ? "…" : scopeEstimate}
                    {settings.dryRun ? " · dry-run" : ""}
                    {settings.keepSource ? " · keep source" : ""}
                  </span>
                </div>
              </fieldset>

              {(busy || resultText) && (
                <fieldset>
                  <legend>Progress</legend>
                  {busy && (
                    <div className="progress-indicator">
                      <span
                        className="progress-indicator-bar"
                        style={{ width: `${progressPct}%` }}
                      />
                    </div>
                  )}
                  {resultText && <p className="result-text">{resultText}</p>}
                </fieldset>
              )}

              <div className="button-row">
                <button
                  type="button"
                  disabled={!canConvert}
                  onClick={startConvert}
                >
                  {converting ? "Cache…" : "Cache"}
                </button>
                <button
                  type="button"
                  disabled={!canConvert}
                  onClick={startStagedCopy}
                >
                  {staging ? "Copy…" : "Staged"}
                </button>
                <button
                  type="button"
                  className="default"
                  disabled={!canConvert}
                  onClick={startInPlace}
                >
                  {inplace
                    ? "USB…"
                    : settings.dryRun
                      ? "Dry run"
                      : "Convert USB"}
                </button>
                <button
                  type="button"
                  disabled={!canInspect}
                  onClick={loadInspect}
                >
                  Inspect
                </button>
              </div>
            </>
          )}

          {tab === "inspect" && (
            <>
              {inspectState.state === "idle" && (
                <p>Select a USB on Convert, then click Inspect.</p>
              )}
              {inspectState.state === "loading" && <p>Reading export.pdb…</p>}
              {inspectState.state === "error" && (
                <p className="error-text">{inspectState.message}</p>
              )}
              {inspectState.state === "ready" && (
                <>
                  <fieldset>
                    <legend>Summary</legend>
                    <div className="field-row">
                      <label>Tracks</label>
                      <span className="value">
                        {inspectState.data.summary.trackCount}
                      </span>
                    </div>
                    <div className="field-row">
                      <label>Playlists</label>
                      <span className="value">
                        {inspectState.data.summary.playlistCount}
                      </span>
                    </div>
                    <div className="field-row">
                      <label>FLAC in DB</label>
                      <span className="value">
                        {inspectState.data.summary.flacInDb}
                      </span>
                    </div>
                    <div className="field-row">
                      <label>Round-trip</label>
                      <span className="value">
                        {inspectState.data.roundtrip.rekordboxPdb
                          .byteIdentical
                          ? "byte-identical"
                          : "differs"}
                      </span>
                    </div>
                  </fieldset>
                  <fieldset>
                    <legend>Dump</legend>
                    <div className="sunken-panel dump-panel">
                      <pre>{inspectState.data.textDump}</pre>
                    </div>
                    <div className="button-row">
                      <button
                        type="button"
                        onClick={() =>
                          navigator.clipboard.writeText(
                            inspectState.data.textDump,
                          )
                        }
                      >
                        Copy
                      </button>
                      <button type="button" onClick={() => setTab("convert")}>
                        Close
                      </button>
                    </div>
                  </fieldset>
                </>
              )}
            </>
          )}

          {tab === "settings" && (
            <>
              <fieldset>
                <legend>Conversion</legend>
                <div className="field-row">
                  <label htmlFor="bitrate">MP3 bitrate</label>
                  <select
                    id="bitrate"
                    value={settingsDraft.bitrate}
                    onChange={(e) =>
                      setSettingsDraft({
                        ...settingsDraft,
                        bitrate: e.target.value,
                      })
                    }
                  >
                    <option value="192k">192k</option>
                    <option value="256k">256k</option>
                    <option value="320k">320k</option>
                  </select>
                </div>
                <div className="field-row">
                  <label htmlFor="workers">FFmpeg threads</label>
                  <select
                    id="workers"
                    value={String(settingsDraft.workers)}
                    onChange={(e) =>
                      setSettingsDraft({
                        ...settingsDraft,
                        workers: Number(e.target.value),
                      })
                    }
                  >
                    <option value="0">Auto</option>
                    {[1, 2, 3, 4, 6, 8].map((n) => (
                      <option key={n} value={String(n)}>
                        {n}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="field-row stacked-control">
                  <label htmlFor="cache">Cache location</label>
                  <input
                    id="cache"
                    type="text"
                    value={settingsDraft.cacheRoot}
                    placeholder={defaultCacheRoot || "Default"}
                    onChange={(e) =>
                      setSettingsDraft({
                        ...settingsDraft,
                        cacheRoot: e.target.value,
                      })
                    }
                  />
                </div>
              </fieldset>

              <fieldset>
                <legend>USB safety</legend>
                <div className="field-row">
                  <input
                    checked={settingsDraft.keepSource}
                    type="checkbox"
                    id="keepSource"
                    onChange={(e) =>
                      setSettingsDraft({
                        ...settingsDraft,
                        keepSource: e.target.checked,
                      })
                    }
                  />
                  <label htmlFor="keepSource">Keep source files on USB</label>
                </div>
                <div className="field-row">
                  <input
                    checked={settingsDraft.verifyOutput}
                    type="checkbox"
                    id="verify"
                    onChange={(e) =>
                      setSettingsDraft({
                        ...settingsDraft,
                        verifyOutput: e.target.checked,
                      })
                    }
                  />
                  <label htmlFor="verify">Verify output before delete</label>
                </div>
                <div className="field-row">
                  <input
                    checked={settingsDraft.dryRun}
                    type="checkbox"
                    id="dryRun"
                    onChange={(e) =>
                      setSettingsDraft({
                        ...settingsDraft,
                        dryRun: e.target.checked,
                      })
                    }
                  />
                  <label htmlFor="dryRun">Dry run (no USB writes)</label>
                </div>
              </fieldset>

              {stagedParent && (
                <p className="hint">Staged copies: {stagedParent}</p>
              )}
              {settingsMsg && <p className="hint">{settingsMsg}</p>}

              <div className="button-row">
                <button type="button" className="default" onClick={saveSettings}>
                  OK
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setSettingsDraft(settings);
                    setSettingsMsg("");
                    setTab("convert");
                  }}
                >
                  Cancel
                </button>
              </div>
            </>
          )}
        </div>
      </div>

      <div className="status-bar">
        <p className="status-bar-field">{statusLine}</p>
        <p className="status-bar-field">v{appVersion}</p>
      </div>

      {confirmInPlace && (
        <div
          className="modal-backdrop"
          onClick={() => setConfirmInPlace(null)}
        >
          <div
            className="window modal-window"
            role="dialog"
            aria-modal="true"
            aria-labelledby="confirm-title"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="title-bar">
              <div className="title-bar-text" id="confirm-title">
                Confirm
              </div>
              <div className="title-bar-controls">
                <button
                  type="button"
                  aria-label="Close"
                  onClick={() => setConfirmInPlace(null)}
                />
              </div>
            </div>
            <div className="window-body">
              <p>
                Convert {confirmInPlace.scopeLabel} in place on{" "}
                {confirmInPlace.root}?
              </p>
              <p>
                Rewrites the USB database and replaces sources with MP3s.
                Failure before source delete rolls the DB back.
              </p>
              <div className="button-row">
                <button type="button" onClick={() => setConfirmInPlace(null)}>
                  Cancel
                </button>
                <button
                  ref={confirmDefaultRef}
                  type="button"
                  className="default"
                  onClick={() => void runInPlace()}
                >
                  Convert USB
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
