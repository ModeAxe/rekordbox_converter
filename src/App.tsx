import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => setAppVersion("?"));
    invoke<string>("ffmpeg_version")
      .then((version) => setFfmpeg({ state: "ok", version }))
      .catch((err) => setFfmpeg({ state: "error", message: String(err) }));
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
      return;
    }

    let cancelled = false;
    setScanState({ state: "scanning" });
    setInspectState({ state: "idle" });

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
      } catch (err) {
        if (cancelled) return;
        setScanState({ state: "error", message: String(err) });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [selectedRoot]);

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

  const selected = drives.find((d) => d.root === selectedRoot);
  const canConvert =
    scanState.state === "ready" && scanState.scan.flacCount > 0;
  const canInspect = selected?.isRekordboxExport ?? false;

  if (view === "inspect") {
    return (
      <main className="app inspect-view">
        <header className="header">
          <h1>Database Inspect</h1>
          <button className="link-btn" type="button" onClick={() => setView("main")}>
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
                  {inspectState.data.roundtrip.rekordcrate.trackCount != null &&
                    ` (${inspectState.data.roundtrip.rekordcrate.trackCount} tracks)`}
                </span>
              </div>
              <div className="status-row">
                <span className="status-label">Writer</span>
                <span className="status-value ok">
                  {inspectState.data.roundtrip.recommended}
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
                <div className="stat">
                  <span className="stat-value">
                    {inspectState.data.summary.mp3InDb}
                  </span>
                  <span className="stat-label">MP3 in DB</span>
                </div>
              </div>
              <p className="hint muted">
                ANLZ files: {inspectState.data.summary.anlzFiles}
                {inspectState.data.summary.anlzPpathMismatches > 0 &&
                  ` · ${inspectState.data.summary.anlzPpathMismatches} PPTH mismatches`}
                {inspectState.data.summary.hasExportExt &&
                  ` · My Tags: ${inspectState.data.summary.myTagCount}`}
              </p>
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

        <footer className="footer">Phase 2 — compare dump with Rekordbox</footer>
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
        {scanState.state === "ready" && (
          <div className="status-row">
            <span className="status-label">exportExt</span>
            <span
              className={
                scanState.scan.hasExportExt
                  ? "status-value ok"
                  : "status-value muted"
              }
            >
              {scanState.scan.hasExportExt ? "✔ found" : "not present"}
            </span>
          </div>
        )}
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
            {scanState.scan.otherCount > 0 && (
              <div className="stat">
                <span className="stat-value">{scanState.scan.otherCount}</span>
                <span className="stat-label">other</span>
              </div>
            )}
          </div>
        )}
      </section>

      <section className="card">
        <h2>Estimated conversion</h2>
        <p className="estimate">
          {scanState.state === "ready"
            ? scanState.scan.flacCount === 0
              ? "Nothing to convert"
              : scanState.estimate
            : "—"}
        </p>
        <button className="convert-btn" disabled={!canConvert} type="button">
          Convert
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
          Conversion in Phase 3. Use Inspect to verify playlists and metadata.
        </p>
      </section>

      <footer className="footer">Phase 2 — USB scan &amp; database inspect</footer>
    </main>
  );
}
