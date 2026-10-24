//! Research: a self-hosted Docent, fed by the verifiers watcher, kept by the
//! engine so evaluation runs reach it with nothing to start by hand.
//!
//! Off by default. `{data_dir}/research.json` turns it on for the device:
//!
//! ```json
//! { "enabled": true, "docentDir": "/path/to/docent", "apiKey": "dk_…" }
//! ```
//!
//! It holds Docent's API key, so it is written owner-only, as `session.json`
//! is; the key reaches the watcher through its environment, never its
//! arguments. Optional fields: `composeFiles` (compose files in `docentDir`,
//! default `docker-compose.yml`), `watcher` (the command, default `python3 -m
//! docent.loaders.watch_verifiers`, run in `docentDir`), `serverUrl` and
//! `webUrl` (Docent's API and UI, default `http://localhost:8889` and `:3001`).
//!
//! When on, the engine:
//!
//! - brings Docent up once with `docker compose up -d`. Docent outlives the
//!   engine: its database is the record, and a restart should not wait on it;
//! - runs the watcher as its child (`kill_on_drop`), so it stops with the
//!   engine, restarting it if it exits; its output goes to
//!   `{data_dir}/research/watcher.log`;
//! - writes the watcher's roots to `{data_dir}/research/roots`, which the
//!   watcher rereads on every pass: every space folder on this device, every
//!   linked worktree of those that are repositories, and every folder a chat
//!   on this device runs in. A new worktree needs no restart.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

use crate::repos::Repos;
use crate::workspace_host::WorkspaceHost;

const SETTINGS_FILE: &str = "research.json";
/// How often the roots file is brought up to date and the watcher checked on.
const TICK: Duration = Duration::from_secs(5);
/// The longest wait before starting a watcher that keeps exiting.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Settings {
    enabled: bool,
    docent_dir: Option<PathBuf>,
    compose_files: Vec<String>,
    watcher: Vec<String>,
    server_url: String,
    web_url: String,
    api_key: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            docent_dir: None,
            compose_files: vec!["docker-compose.yml".into()],
            watcher: ["python3", "-m", "docent.loaders.watch_verifiers"]
                .map(String::from)
                .to_vec(),
            server_url: "http://localhost:8889".into(),
            web_url: "http://localhost:3001".into(),
            api_key: None,
        }
    }
}

impl Settings {
    /// The device's settings, or `None` when Research is off or cannot run. A
    /// missing file is off; an unreadable one is off with a warning.
    fn load(data_dir: &Path) -> Option<(Self, PathBuf)> {
        let path = data_dir.join(SETTINGS_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "research: settings unreadable");
                return None;
            }
        };
        let settings: Self = match serde_json::from_str(&text) {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "research: settings invalid");
                return None;
            }
        };
        if !settings.enabled {
            return None;
        }
        let Some(docent_dir) = settings.docent_dir.clone() else {
            tracing::warn!("research: on, but no docentDir; not starting");
            return None;
        };
        if settings.watcher.is_empty() {
            tracing::warn!("research: on, but the watcher command is empty; not starting");
            return None;
        }
        Some((settings, docent_dir))
    }
}

/// Research's processes for one engine runtime. Dropping it stops the watcher.
pub struct Research {
    cancel: CancellationToken,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Research {
    /// Start Research if the device has it on. Requires a tokio runtime.
    pub fn start(
        data_dir: &Path,
        repos: Repos,
        workspace: WorkspaceHost,
        device_id: String,
    ) -> Option<Self> {
        let (settings, docent_dir) = Settings::load(data_dir)?;
        let roots_file = data_dir.join("research").join("roots");
        let cancel = CancellationToken::new();
        let keeper = Keeper {
            settings,
            docent_dir,
            roots_file,
            repos,
            workspace,
            device_id,
        };
        let task = tokio::spawn(keeper.run(cancel.clone()));
        tracing::info!("research: started");
        Some(Self {
            cancel,
            task: Mutex::new(Some(task)),
        })
    }

    /// Stop the watcher and wait for it to go. Docent is left running.
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let task = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

impl Drop for Research {
    fn drop(&mut self) {
        // The task owns the child; aborting drops it, and `kill_on_drop` ends it.
        if let Some(task) = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}

struct Keeper {
    settings: Settings,
    docent_dir: PathBuf,
    roots_file: PathBuf,
    repos: Repos,
    workspace: WorkspaceHost,
    device_id: String,
}

impl Keeper {
    async fn run(self, cancel: CancellationToken) {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = self.compose_up() => {}
        }
        let mut watcher: Option<Child> = None;
        let mut backoff = TICK;
        let mut next_start = tokio::time::Instant::now();
        let mut written: Option<Vec<PathBuf>> = None;
        loop {
            let roots = self.roots().await;
            if written.as_ref() != Some(&roots) {
                match write_roots(&self.roots_file, &roots) {
                    Ok(()) => written = Some(roots),
                    Err(err) => {
                        tracing::warn!(error = %err, "research: couldn't write the roots file")
                    }
                }
            }
            if let Some(child) = watcher.as_mut()
                && let Ok(Some(status)) = child.try_wait()
            {
                tracing::warn!(%status, "research: the watcher exited");
                watcher = None;
                next_start = tokio::time::Instant::now() + backoff;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
            if watcher.is_none() && tokio::time::Instant::now() >= next_start {
                match self.spawn_watcher() {
                    Ok(child) => watcher = Some(child),
                    Err(err) => {
                        tracing::warn!(error = %err, "research: couldn't start the watcher");
                        next_start = tokio::time::Instant::now() + backoff;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    }
                }
            }
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(TICK) => {}
            }
        }
        if let Some(mut child) = watcher {
            let _ = child.kill().await;
        }
    }

    /// `docker compose up -d`, once. A failure is logged and the watcher still
    /// starts: Docent may be up already, or come up later, and the watcher
    /// keeps every line until Docent takes it.
    async fn compose_up(&self) {
        let mut command = Command::new("docker");
        command.arg("compose");
        for file in &self.settings.compose_files {
            command.arg("-f").arg(file);
        }
        command
            .args(["up", "-d"])
            .current_dir(&self.docent_dir)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        match command.output().await {
            Ok(out) if out.status.success() => tracing::info!("research: Docent is up"),
            Ok(out) => tracing::warn!(
                status = %out.status,
                stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                "research: docker compose up failed"
            ),
            Err(err) => tracing::warn!(error = %err, "research: couldn't run docker compose"),
        }
    }

    fn spawn_watcher(&self) -> std::io::Result<Child> {
        let (program, args) = self
            .settings
            .watcher
            .split_first()
            .ok_or_else(|| std::io::Error::other("empty watcher command"))?;
        // Its own log beside the roots file: the engine's stderr may go nowhere.
        let log_path = self.roots_file.with_file_name("watcher.log");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        let mut command = Command::new(program);
        command
            .args(args)
            .stdout(log.try_clone()?)
            .stderr(log)
            .arg("--roots-file")
            .arg(&self.roots_file)
            .args(["--server-url", &self.settings.server_url])
            .args(["--web-url", &self.settings.web_url])
            .current_dir(&self.docent_dir)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        if let Some(key) = &self.settings.api_key {
            command.env("DOCENT_API_KEY", key);
        }
        let child = command.spawn()?;
        tracing::info!(pid = child.id(), "research: watcher started");
        Ok(child)
    }

    /// Every folder a run on this device could be written under, sorted.
    async fn roots(&self) -> Vec<PathBuf> {
        let mut roots = BTreeSet::new();
        let spaces = self.workspace.read_spaces().unwrap_or_default();
        for space in spaces.into_iter().filter(|s| s.device_id == self.device_id) {
            let path = PathBuf::from(&space.path);
            if space.git_detected
                && let Ok(worktrees) = self.repos.worktree_paths(&path).await
            {
                roots.extend(worktrees);
            }
            roots.insert(path);
        }
        let chats = self.workspace.read_chats().unwrap_or_default();
        roots.extend(
            chats
                .into_iter()
                .filter(|chat| chat.device_id == self.device_id)
                .filter_map(|chat| chat.cwd.map(PathBuf::from)),
        );
        roots.into_iter().collect()
    }
}

/// Replace the roots file whole, so the watcher never reads half of it.
fn write_roots(path: &Path, roots: &[PathBuf]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("roots file has no folder"))?;
    std::fs::create_dir_all(dir)?;
    let mut text = String::from("# Written by the Zeron engine; reread by the watcher.\n");
    for root in roots {
        let line = root.to_string_lossy();
        // A path the file format can't carry (a newline in it) is left out.
        if !line.contains('\n') {
            text.push_str(&line);
            text.push('\n');
        }
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn research_is_off_without_a_settings_file_or_when_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(Settings::load(tmp.path()).is_none());
        std::fs::write(tmp.path().join(SETTINGS_FILE), r#"{"docentDir": "/d"}"#).unwrap();
        assert!(Settings::load(tmp.path()).is_none());
        std::fs::write(tmp.path().join(SETTINGS_FILE), r#"{"enabled": true}"#).unwrap();
        assert!(Settings::load(tmp.path()).is_none(), "on but no docentDir");
    }

    #[test]
    fn settings_fill_in_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(SETTINGS_FILE),
            r#"{"enabled": true, "docentDir": "/d", "composeFiles": ["a.yml", "b.yml"]}"#,
        )
        .unwrap();
        let (settings, dir) = Settings::load(tmp.path()).unwrap();
        assert_eq!(dir, PathBuf::from("/d"));
        assert_eq!(settings.compose_files, ["a.yml", "b.yml"]);
        assert_eq!(settings.watcher[0], "python3");
        assert_eq!(settings.server_url, "http://localhost:8889");
        assert_eq!(settings.api_key, None);
    }

    #[test]
    fn the_roots_file_lists_one_root_per_line_and_is_replaced_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("research").join("roots");
        write_roots(&path, &[PathBuf::from("/a"), PathBuf::from("/b\nc")]).unwrap();
        write_roots(&path, &[PathBuf::from("/a"), PathBuf::from("/d")]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let roots: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(roots, ["/a", "/d"]);
        assert!(!path.with_extension("tmp").exists());
    }
}
