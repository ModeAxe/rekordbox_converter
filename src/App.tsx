import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";

type FfmpegStatus =
  | { state: "checking" }
  | { state: "ok"; version: string }
  | { state: "error"; message: string };

export default function App() {
  const [appVersion, setAppVersion] = useState<string>("");
  const [ffmpeg, setFfmpeg] = useState<FfmpegStatus>({ state: "checking" });

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => setAppVersion("?"));
    invoke<string>("ffmpeg_version")
      .then((version) => setFfmpeg({ state: "ok", version }))
      .catch((err) => setFfmpeg({ state: "error", message: String(err) }));
  }, []);

  return (
    <main className="app">
      <header className="header">
        <h1>Rekordbox USB Converter</h1>
        <span className="version">v{appVersion}</span>
      </header>

      <section className="card">
        <h2>Status</h2>
        <div className="status-row">
          <span className="status-label">FFmpeg</span>
          {ffmpeg.state === "checking" && (
            <span className="status-value muted">checking…</span>
          )}
          {ffmpeg.state === "ok" && (
            <span className="status-value ok" title={ffmpeg.version}>
              ✔ {ffmpeg.version}
            </span>
          )}
          {ffmpeg.state === "error" && (
            <span className="status-value error" title={ffmpeg.message}>
              ✘ not available — {ffmpeg.message}
            </span>
          )}
        </div>
        <div className="status-row">
          <span className="status-label">USB</span>
          <span className="status-value muted">
            detection arrives in Phase 1
          </span>
        </div>
      </section>

      <footer className="footer">
        Phase 0 scaffold — conversion features are added phase by phase.
      </footer>
    </main>
  );
}
