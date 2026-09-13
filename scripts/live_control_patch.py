from pathlib import Path


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


lib_path = Path("src-tauri/src/lib.rs")
lib = lib_path.read_text()

if "mod download_control;" not in lib:
    anchor = "use lumina_core::{"
    require(anchor in lib, "missing lib import anchor")
    lib = lib.replace(
        anchor,
        "mod download_control;\n\nuse download_control::ControlledDownload;\nuse lumina_core::{",
        1,
    )

if "async fn pause_download(" not in lib:
    anchor = "#[tauri::command]\nasync fn resolver_policy"
    require(anchor in lib, "missing resolver command anchor")
    controls = '''#[tauri::command]
async fn pause_download(state: State<'_, AppState>, task_id: String) -> Result<bool, String> {
    let accepted = download_control::pause(&task_id).await;
    if accepted {
        if let Some(task) = state
            .store
            .list_download_tasks()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|task| task.id == task_id)
        {
            state
                .store
                .update_download_task(
                    &task_id,
                    DownloadState::Paused,
                    task.downloaded_bytes,
                    task.total_bytes,
                    None,
                )
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(accepted)
}

#[tauri::command]
async fn cancel_download(state: State<'_, AppState>, task_id: String) -> Result<bool, String> {
    let accepted = download_control::cancel(&task_id).await;
    if accepted {
        if let Some(task) = state
            .store
            .list_download_tasks()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|task| task.id == task_id)
        {
            state
                .store
                .update_download_task(
                    &task_id,
                    DownloadState::Cancelled,
                    task.downloaded_bytes,
                    task.total_bytes,
                    None,
                )
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(accepted)
}

'''
    lib = lib.replace(anchor, controls + anchor, 1)

download_fn = lib.index("async fn download_book(")
if "download_control::run_controlled(" not in lib[download_fn:]:
    result_start = lib.index("    let result = downloader\n", download_fn)
    item_start = lib.index("    let item = LibraryItem::inspect", result_start)
    controlled = '''    let result = download_control::run_controlled(
        task_id.clone(),
        downloader.download_to_with_context(
            url,
            destination.clone(),
            headers,
            Some(format!("{provider_id}:{book_id}")),
            tx,
        ),
    )
    .await;
    let _ = monitor.await;

    let latest = state
        .store
        .list_download_tasks()
        .ok()
        .and_then(|tasks| tasks.into_iter().find(|task| task.id == task_id));
    let downloaded = latest
        .as_ref()
        .map_or(resume_downloaded, |task| task.downloaded_bytes);
    let total = latest
        .as_ref()
        .and_then(|task| task.total_bytes)
        .or(resume_total);

    match result {
        ControlledDownload::Paused => {
            state
                .store
                .update_download_task(
                    &task_id,
                    DownloadState::Paused,
                    downloaded,
                    total,
                    None,
                )
                .map_err(|error| error.to_string())?;
            return Err("__LUMINA_PAUSED__".to_string());
        }
        ControlledDownload::Cancelled => {
            state
                .store
                .update_download_task(
                    &task_id,
                    DownloadState::Cancelled,
                    downloaded,
                    total,
                    None,
                )
                .map_err(|error| error.to_string())?;
            return Err("__LUMINA_CANCELLED__".to_string());
        }
        ControlledDownload::AlreadyActive => {
            return Err("download task is already active".to_string());
        }
        ControlledDownload::Finished(Err(error)) => {
            let message = error.to_string();
            let _ = state.store.update_download_task(
                &task_id,
                DownloadState::Failed,
                downloaded,
                total,
                Some(&message),
            );
            return Err(message);
        }
        ControlledDownload::Finished(Ok(())) => {}
    }

'''
    lib = lib[:result_start] + controlled + lib[item_start:]

if "            pause_download," not in lib:
    anchor = "            remove_download_task,\n            resolver_policy,"
    require(anchor in lib, "missing invoke handler anchor")
    lib = lib.replace(
        anchor,
        "            remove_download_task,\n            pause_download,\n            cancel_download,\n            resolver_policy,",
        1,
    )

lib_path.write_text(lib)

app_path = Path("src/App.tsx")
app = app_path.read_text()

if "async function refreshDownloads()" not in app:
    anchor = "  async function runSearch(targetPage = 1) {"
    pos = app.index(anchor)
    helper = '''  async function refreshDownloads() {
    const persisted = await invoke<PersistedDownloadTask[]>("download_tasks");
    setDownloads(restoreDownloads(persisted));
  }

'''
    app = app[:pos] + helper + app[pos:]

queue_start = app.index("  async function queueDownload(")
start_download = app.index("  async function startDownload", queue_start)
queue_block = app[queue_start:start_download]
if "__LUMINA_PAUSED__" not in queue_block:
    catch_start = queue_start + queue_block.index("    } catch (reason) {")
    catch_end = app.index("\n    }\n  }", catch_start)
    new_catch = '''    } catch (reason) {
      const message = String(reason);
      try {
        await refreshDownloads();
      } catch (refreshReason) {
        setError(`${message} · 队列刷新失败：${String(refreshReason)}`);
        return;
      }
      if (message.includes("__LUMINA_PAUSED__") || message.includes("__LUMINA_CANCELLED__")) {
        return;
      }
      setError(message);
'''
    app = app[:catch_start] + new_catch + app[catch_end:]

if "async function pauseDownload(task: DownloadTask)" not in app:
    pos = app.index("  async function removeDownload(task: DownloadTask) {")
    controls = '''  async function pauseDownload(task: DownloadTask) {
    try {
      const accepted = await invoke<boolean>("pause_download", { taskId: task.key });
      if (!accepted) {
        await refreshDownloads();
        return;
      }
      setDownloads((current) => ({
        ...current,
        [task.key]: { ...current[task.key], state: "paused" },
      }));
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function cancelDownload(task: DownloadTask) {
    try {
      const accepted = await invoke<boolean>("cancel_download", { taskId: task.key });
      if (!accepted) {
        await refreshDownloads();
        return;
      }
      setDownloads((current) => ({
        ...current,
        [task.key]: { ...current[task.key], state: "cancelled" },
      }));
    } catch (reason) {
      setError(String(reason));
    }
  }

'''
    app = app[:pos] + controls + app[pos:]

render_start = app.index("  function renderDownloads()")
render_end = app.index("  function renderLibrary()", render_start)
render_block = app[render_start:render_end]
if "pauseDownload(task)" not in render_block:
    actions_start = render_start + render_block.index('<div className="download-actions">')
    actions_end = app.index("</div>", actions_start) + len("</div>")
    replacement = '''<div className="download-actions">
                    {task.state === "downloading" ? <button className="ghost-button" onClick={() => void pauseDownload(task)}>暂停</button> : null}
                    {task.state === "downloading" ? <button className="ghost-button danger" onClick={() => void cancelDownload(task)}>取消</button> : null}
                    {canResume ? <button className="primary-button small" onClick={() => void retryDownload(task)}>继续</button> : null}
                    {!isActive ? <button className="ghost-button" onClick={() => void removeDownload(task)}>移除记录</button> : null}
                  </div>'''
    app = app[:actions_start] + replacement + app[actions_end:]

app_path.write_text(app)
