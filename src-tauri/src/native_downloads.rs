#[cfg(target_os = "android")]
use super::{download_control::TransferControl, AppState};
use lumina_core::PersistedDownloadTask;
use tauri::AppHandle;
#[cfg(target_os = "android")]
use tauri::{plugin::PluginHandle, Manager};

#[cfg(target_os = "android")]
struct AndroidTransfers(PluginHandle<tauri::Wry>);

pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("download-runtime")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                let handle = _api
                    .register_android_plugin("app.luminashelf.client", "DownloadRuntimePlugin")?;
                _app.manage(AndroidTransfers(handle));
            }
            Ok(())
        })
        .build()
}

pub async fn start(app: &AppHandle, task: &PersistedDownloadTask) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        #[derive(serde::Deserialize)]
        struct Control {
            id: String,
            action: String,
        }
        let app_handle = app.clone();
        let expected_id = task.id.clone();
        let channel = tauri::ipc::Channel::<Control>::new(move |body| {
            let message: Control = body.deserialize()?;
            if message.id == expected_id && message.action == "pause" {
                let handle = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<AppState>();
                    let _ = state
                        .transfers
                        .stop(&message.id, TransferControl::Paused)
                        .await;
                });
            }
            Ok(())
        });
        app.state::<AndroidTransfers>()
            .0
            .run_mobile_plugin_async::<serde_json::Value>(
                "start",
                serde_json::json!({ "id": task.id, "title": task.title, "onControl": channel }),
            )
            .await
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    let _ = (app, task);
    Ok(())
}

pub fn update(app: &AppHandle, task: &PersistedDownloadTask) {
    #[cfg(target_os = "android")]
    {
        let progress = task
            .total_bytes
            .filter(|total| *total > 0)
            .map(|total| {
                ((task.downloaded_bytes as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as i32
            })
            .unwrap_or(-1);
        let _ = app.state::<AndroidTransfers>().0.run_mobile_plugin::<serde_json::Value>(
            "update", serde_json::json!({ "id": task.id, "title": task.title, "progress": progress, "active": task.state.is_active() })
        );
    }
    #[cfg(not(target_os = "android"))]
    let _ = (app, task);
}

pub async fn share(app: &AppHandle, path: &std::path::Path, mime: &str) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        app.state::<AndroidTransfers>()
            .0
            .run_mobile_plugin_async::<serde_json::Value>(
                "share",
                serde_json::json!({ "path": path.to_string_lossy(), "mime": mime }),
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, path, mime);
        Err("请使用导出电子书保存副本。".into())
    }
}
