import { useEffect, useState } from "react";
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
  flacCount: number;
  mp3Count: number;
  otherCount: number;
  flacBytes: number;
  estimatedSeconds: number;
  flacPaths: string[];
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
  flacsRemoved: number;
  verifyProblems: string[];
  errors: string[];
};

type PipelineState =
  | { state: "idle" }
  | { state: "running"; progress: PipelineProgress | null }
  | { state: "done"; summary: StagedCopySummary }
  | { state: "error"; message: string };

type PlaylistOption = {
  id: number;
  name: string;
  depth: number;
  trackCount: number;
  flacCount: number;
  mp3Count: number;
  otherCount: number;
};

type ConvertScope = {
  playlistId: number | null;
  playlistName: string | null;
  flacPaths: string[];
  flacCount: number;
  estimatedSeconds: number;
};

/** Sentinel for "all FLACs on the USB". */
const PLAYLIST_ALL = "";

const POLL_MS = 2500;

export default function App() {
  const [appVersion, setAppVersion] = useState<string>("");
  const [ffmpeg, setFfmpeg] = useState<FfmpegStatus>({ state: "checking" });
  const [drives, setDrives] = useState<DriveInfo[]>([]);
  const [selectedRoot, setSelectedRoot] = useState<string>("");
  const [scanState, setScanState] = useState<ScanState>({ state: "idle" });
  const [view, setView] = useState<"main" | "inspect">("main");
  const [inspectState, setInspectState] = useState<InspectState>({
    state: "idle",
  });
  const [convertState, setConvertState] = useState<ConvertState>({
    state: "idle",
  });
  const [pipelineState, setPipelineState] = useState<PipelineState>({
    state: "idle",
  });
  const [cacheRoot, setCacheRoot] = useState<string>("");
  const [stagedParent, setStagedParent] = useState<string>("");
  const [playlists, setPlaylists] = useState<PlaylistOption[]>([]);
  const [playlistSelect, setPlaylistSelect] = useState<string>(PLAYLIST_ALL);
  const [scope, setScope] = useState<ConvertScope | null>(null);
  const [scopeEstimate, setScopeEstimate] = useState<string>("—");
  const [scopeLoading, setScopeLoading] = useState(false);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => setAppVersion("?"));
    invoke<string>("ffmpeg_version")
      .then((version) => setFfmpeg({ state: "ok", version }))
      .catch((err) => setFfmpeg({ state: "error", message: String(err) }));
    invoke<{ root: string }>("get_cache_root")
      .then((info) => setCacheRoot(info.root))
      .catch(() => setCacheRoot(""));
    invoke<{ root: string }>("get_staged_parent")
      .then((info) => setStagedParent(info.root))
      .catch(() => setStagedParent(""));
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
        /* keep previous list */
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
      setInspectState({ state: "idle" });
      setPlaylists([]);
      setPlaylistSelect(PLAYLIST_ALL);
      setScope(null);
      return;
    }

    let cancelled = false;
    setScanState({ state: "scanning" });
    setInspectState({ state: "idle" });
    setConvertState({ state: "idle" });
    setPipelineState({ state: "idle" });
    setPlaylists([]);
    setPlaylistSelect(PLAYLIST_ALL);

    (async () => {
      try {
        const scan = await invoke<ExportScan>("scan_drive", {
          root: selectedRoot,
        });
        if (cancelled) return;
        const estimate = await invoke<string>("format_estimate", {
          seconds: scan.estimatedSeconds,
        });
        if (cancelled) return;
        setScanState({ state: "ready", scan, estimate });

        try {
          const pls = await invoke<PlaylistOption[]>("list_export_playlists", {
            root: selectedRoot,
          });
          if (!cancelled) setPlaylists(pls);
        } catch {
          if (!cancelled) setPlaylists([]);
        }
      } catch (err) {
        if (cancelled) return;
        setScanState({ state: "error", message: String(err) });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [selectedRoot]);

  // Refresh convert scope whenever drive or playlist selection changes.
  useEffect(() => {
    if (!selectedRoot || scanState.state !== "ready") {
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
        if (cancelled) return;
        setScope(next);
        const estimate = await invoke<string>("format_estimate", {
          seconds: next.estimatedSeconds,
        });
        if (cancelled) return;
        setScopeEstimate(
          next.flacCount === 0 ? "Nothing to convert" : estimate,
        );
      } catch (err) {
        if (cancelled) return;
        setScope(null);
        setScopeEstimate("—");
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
    let unlistenProgress: (() => void) | undefined;
    let unlistenFinished: (() => void) | undefined;
    let unlistenPipelineProgress: (() => void) | undefined;
    let unlistenPipelineFinished: (() => void) | undefined;

    listen<ConvertProgress>("conversion-progress", (event) => {
      setConvertState({ state: "running", progress: event.payload });
    }).then((fn) => {
      unlistenProgress = fn;
    });

    listen<ConvertSummary>("conversion-finished", (event) => {
      setConvertState({ state: "done", summary: event.payload });
    }).then((fn) => {
      unlistenFinished = fn;
    });

    listen<PipelineProgress>("pipeline-progress", (event) => {
      setPipelineState({ state: "running", progress: event.payload });
    }).then((fn) => {
      unlistenPipelineProgress = fn;
    });

    listen<StagedCopySummary>("pipeline-finished", (event) => {
      setPipelineState({ state: "done", summary: event.payload });
    }).then((fn) => {
      unlistenPipelineFinished = fn;
    });

    return () => {
      unlistenProgress?.();
      unlistenFinished?.();
      unlistenPipelineProgress?.();
      unlistenPipelineFinished?.();
    };
  }, []);

  const loadInspect = async () => {
    if (!selectedRoot) return;
    setInspectState({ state: "loading" });
    setView("inspect");
    try {
      const data = await invoke<ExportInspect>("inspect_drive", {
        root: selectedRoot,
      });
      setInspectState({ state: "ready", data });
    } catch (err) {
      setInspectState({ state: "error", message: String(err) });
    }
  };

  const startConvert = async () => {
    if (!selectedRoot) return;
    setConvertState({ state: "running", progress: null });
    const playlistId =
      playlistSelect === PLAYLIST_ALL ? null : Number(playlistSelect);
    try {
      const summary = await invoke<ConvertSummary>("convert_to_cache", {
        root: selectedRoot,
        playlistId,
        workers: null,
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
        workers: null,
      });
      setPipelineState({ state: "done", summary });
    } catch (err) {
      setPipelineState({ state: "error", message: String(err) });
    }
  };

  const selected = drives.find((d) => d.root === selectedRoot);
  const converting = convertState.state === "running";
  const staging = pipelineState.state === "running";
  const busy = converting || staging;
  const scopeFlacCount = scope?.flacCount ?? 0;
  const canConvert =
    scanState.state === "ready" &&
    scopeFlacCount > 0 &&
    ffmpeg.state === "ok" &&
    !busy &&
    !scopeLoading;
  const canInspect = (selected?.isRekordboxExport ?? false) && !busy;

  const progressPct =
    convertState.state === "running" && convertState.progress
      ? Math.round(
          (convertState.progress.completed / convertState.progress.total) * 100,
        )
      : convertState.state === "done"
        ? 100
        : 0;

  if (view === "inspect") {
    return (
      <main className="app inspect-view">
        <header className="header">
          <h1>Database Inspect</h1>
          <button
            className="link-btn"
            type="button"
            onClick={() => setView("main")}
          >
            ← Back
          </button>
        </header>

        {inspectState.state === "loading" && (
          <p className="muted empty">Reading export.pdb…</p>
        )}
        {inspectState.state === "error" && (
          <p className="status-value error">{inspectState.message}</p>
        )}
        {inspectState.state === "ready" && (
          <>
            <section className="card">
              <h2>Round-trip proof</h2>
              <div className="status-row">
                <span className="status-label">rekordbox-pdb</span>
                <span
                  className={
                    inspectState.data.roundtrip.rekordboxPdb.byteIdentical
                      ? "status-value ok"
                      : "status-value error"
                  }
                >
                  {inspectState.data.roundtrip.rekordboxPdb.byteIdentical
                    ? "✔ byte-identical"
                    : "✘ differs"}{" "}
                  — {inspectState.data.roundtrip.rekordboxPdb.message}
                </span>
              </div>
              <div className="status-row">
                <span className="status-label">rekordcrate</span>
                <span className="status-value muted">
                  {inspectState.data.roundtrip.rekordcrate.parsed
                    ? "parsed"
                    : "failed"}{" "}
                  — {inspectState.data.roundtrip.rekordcrate.message}
                </span>
              </div>
            </section>

            <section className="card">
              <h2>Summary</h2>
              <div className="stats">
                <div className="stat">
                  <span className="stat-value">
                    {inspectState.data.summary.trackCount}
                  </span>
                  <span className="stat-label">DB tracks</span>
                </div>
                <div className="stat">
                  <span className="stat-value">
                    {inspectState.data.summary.playlistCount}
                  </span>
                  <span className="stat-label">playlists</span>
                </div>
                <div className="stat">
                  <span className="stat-value accent">
                    {inspectState.data.summary.flacInDb}
                  </span>
                  <span className="stat-label">FLAC in DB</span>
                </div>
              </div>
            </section>

            <section className="card">
              <h2>Playlists</h2>
              <ul className="playlist-list">
                {inspectState.data.playlists
                  .filter((p) => !p.isFolder)
                  .map((p) => (
                    <li key={p.id}>
                      {p.name}{" "}
                      <span className="muted">({p.trackCount} tracks)</span>
                    </li>
                  ))}
              </ul>
            </section>

            <section className="card dump-card">
              <div className="dump-header">
                <h2>Full dump</h2>
                <button
                  className="link-btn"
                  type="button"
                  onClick={() =>
                    navigator.clipboard.writeText(inspectState.data.textDump)
                  }
                >
                  Copy
                </button>
              </div>
              <pre className="text-dump">{inspectState.data.textDump}</pre>
            </section>
          </>
        )}

        <footer className="footer">Phase 2 — database inspect</footer>
      </main>
    );
  }

  return (
    <main className="app">
      <header className="header">
        <h1>Rekordbox USB Converter</h1>
        <span className="version">v{appVersion}</span>
      </header>

      <section className="card">
        <h2>USB</h2>
        {drives.length === 0 ? (
          <p className="muted empty">
            No removable drives detected. Plug in a Rekordbox-exported USB.
          </p>
        ) : (
          <select
            className="drive-select"
            value={selectedRoot}
            onChange={(e) => setSelectedRoot(e.target.value)}
            aria-label="USB drive"
            disabled={busy}
          >
            {drives.map((d) => (
              <option key={d.root} value={d.root}>
                {d.label}
                {d.isRekordboxExport ? " — Rekordbox export" : ""}
              </option>
            ))}
          </select>
        )}
      </section>

      <section className="card">
        <h2>Status</h2>
        <div className="status-row">
          <span className="status-label">FFmpeg</span>
          {ffmpeg.state === "checking" && (
            <span className="status-value muted">checking…</span>
          )}
          {ffmpeg.state === "ok" && (
            <span className="status-value ok" title={ffmpeg.version}>
              ✔ ready
            </span>
          )}
          {ffmpeg.state === "error" && (
            <span className="status-value error" title={ffmpeg.message}>
              ✘ not available
            </span>
          )}
        </div>
        <div className="status-row">
          <span className="status-label">USB</span>
          {!selectedRoot && (
            <span className="status-value muted">waiting for drive…</span>
          )}
          {selectedRoot && selected?.isRekordboxExport && (
            <span className="status-value ok">✔ USB detected</span>
          )}
          {selectedRoot && selected && !selected.isRekordboxExport && (
            <span className="status-value error">
              ✘ no export.pdb on this drive
            </span>
          )}
        </div>
      </section>

      <section className="card">
        <h2>Export</h2>
        {scanState.state === "idle" && (
          <p className="muted empty">Select a USB to inspect.</p>
        )}
        {scanState.state === "scanning" && (
          <p className="muted empty">Scanning…</p>
        )}
        {scanState.state === "error" && (
          <p className="status-value error">{scanState.message}</p>
        )}
        {scanState.state === "ready" && (
          <div className="stats">
            <div className="stat">
              <span className="stat-value">{scanState.scan.totalTracks}</span>
              <span className="stat-label">tracks</span>
            </div>
            <div className="stat">
              <span className="stat-value accent">
                {scanState.scan.flacCount}
              </span>
              <span className="stat-label">FLAC</span>
            </div>
            <div className="stat">
              <span className="stat-value">{scanState.scan.mp3Count}</span>
              <span className="stat-label">MP3</span>
            </div>
          </div>
        )}
      </section>

      <section className="card">
        <h2>Convert scope</h2>
        <select
          className="drive-select"
          value={playlistSelect}
          onChange={(e) => setPlaylistSelect(e.target.value)}
          aria-label="Playlist"
            disabled={
            busy ||
            scanState.state !== "ready" ||
            !selected?.isRekordboxExport
          }
        >
          <option value={PLAYLIST_ALL}>
            All FLACs on USB
            {scanState.state === "ready"
              ? ` (${scanState.scan.flacCount})`
              : ""}
          </option>
          {playlists.map((p) => (
            <option key={p.id} value={String(p.id)}>
              {"\u00A0".repeat(p.depth * 2)}
              {p.name} — {p.flacCount} FLAC / {p.trackCount} tracks
            </option>
          ))}
        </select>
        <p className="hint muted" style={{ marginTop: 10 }}>
          Only tracks in the selected playlist are converted. Phase 4+ will
          replace only those on the USB.
        </p>
      </section>

      <section className="card">
        <h2>Estimated conversion</h2>
        <p className="estimate">
          {scopeLoading
            ? "…"
            : scanState.state === "ready"
              ? scopeEstimate
              : "—"}
        </p>
        {scope && playlistSelect !== PLAYLIST_ALL && (
          <p className="hint muted" style={{ marginBottom: 12 }}>
            {scope.playlistName}: {scope.flacCount} FLAC
            {scope.flacCount === 1 ? "" : "s"}
          </p>
        )}
        <button
          className="convert-btn"
          disabled={!canConvert}
          type="button"
          onClick={startConvert}
        >
          {converting ? "Converting…" : "Convert to cache"}
        </button>
        <button
          className="convert-btn staged-btn"
          disabled={!canConvert}
          type="button"
          onClick={startStagedCopy}
        >
          {staging ? "Building copy…" : "Create converted copy"}
        </button>
        <button
          className="secondary-btn"
          disabled={!canInspect}
          type="button"
          onClick={loadInspect}
        >
          Inspect database
        </button>
        <p className="hint muted">
          Cache warms MP3s only. <strong>Create converted copy</strong> writes a
          full staged export (DB + ANLZ rewritten) under a new folder — your USB
          is never touched.
          {cacheRoot && (
            <>
              <br />
              Cache: {cacheRoot}
            </>
          )}
          {stagedParent && (
            <>
              <br />
              Staged copies: {stagedParent}
            </>
          )}
        </p>
      </section>

      {(convertState.state === "running" ||
        convertState.state === "done" ||
        convertState.state === "error") && (
        <section className="card">
          <h2>Progress</h2>
          <div className="progress-bar" aria-valuenow={progressPct}>
            <div
              className="progress-fill"
              style={{ width: `${progressPct}%` }}
            />
          </div>
          {convertState.state === "running" && convertState.progress && (
            <>
              <p className="progress-label">
                Track {convertState.progress.completed} /{" "}
                {convertState.progress.total}
              </p>
              <p className="progress-file">
                {convertState.progress.outcome === "cacheHit"
                  ? "Cache hit: "
                  : convertState.progress.outcome === "failed"
                    ? "Failed: "
                    : "Converting: "}
                {convertState.progress.currentFile}
              </p>
              <p className="hint muted">
                hits {convertState.progress.cacheHits} · converted{" "}
                {convertState.progress.converted} · failed{" "}
                {convertState.progress.failed}
              </p>
            </>
          )}
          {convertState.state === "running" && !convertState.progress && (
            <p className="muted empty">Starting…</p>
          )}
          {convertState.state === "done" && (
            <>
              <p className="status-value ok">
                ✔ Done — {convertState.summary.converted} converted,{" "}
                {convertState.summary.cacheHits} cache hits
                {convertState.summary.failed > 0 &&
                  `, ${convertState.summary.failed} failed`}
              </p>
              <p className="hint muted">
                Cache: {convertState.summary.cacheRoot}
              </p>
              {convertState.summary.errors.length > 0 && (
                <pre className="text-dump">
                  {convertState.summary.errors.join("\n")}
                </pre>
              )}
            </>
          )}
          {convertState.state === "error" && (
            <p className="status-value error">{convertState.message}</p>
          )}
        </section>
      )}

      {(pipelineState.state === "running" ||
        pipelineState.state === "done" ||
        pipelineState.state === "error") && (
        <section className="card">
          <h2>Staged copy</h2>
          {pipelineState.state === "running" && pipelineState.progress && (
            <>
              <p className="progress-label">
                {pipelineState.progress.phase}
                {pipelineState.progress.total > 0 &&
                  ` — ${pipelineState.progress.completed}/${pipelineState.progress.total}`}
              </p>
              <p className="progress-file">{pipelineState.progress.detail}</p>
            </>
          )}
          {pipelineState.state === "running" && !pipelineState.progress && (
            <p className="muted empty">Starting…</p>
          )}
          {pipelineState.state === "done" && (
            <>
              <p className="status-value ok">
                ✔ Copy ready — {pipelineState.summary.pdbUpdated} DB tracks,{" "}
                {pipelineState.summary.anlzUpdated} ANLZ files,{" "}
                {pipelineState.summary.flacsRemoved} FLACs removed from copy
              </p>
              <p className="hint muted">
                Output: {pipelineState.summary.outputRoot}
              </p>
              <p className="hint muted">
                Copy this folder onto a spare USB (as the drive root) and test in
                Rekordbox / on your CDJs.
              </p>
              {pipelineState.summary.verifyProblems.length > 0 && (
                <pre className="text-dump">
                  Verify:{"\n"}
                  {pipelineState.summary.verifyProblems.join("\n")}
                </pre>
              )}
              {pipelineState.summary.errors.length > 0 && (
                <pre className="text-dump">
                  {pipelineState.summary.errors.join("\n")}
                </pre>
              )}
            </>
          )}
          {pipelineState.state === "error" && (
            <p className="status-value error">{pipelineState.message}</p>
          )}
        </section>
      )}

      <footer className="footer">
        Phase 4 — staged converted copy (USB untouched)
      </footer>
    </main>
  );
}
