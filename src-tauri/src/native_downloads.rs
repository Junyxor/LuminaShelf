#[cfg(target_os = "android")]
use super::{download_control::TransferControl, AppState};
use lumina_core::PersistedDownloadTask;
use tauri::AppHandle;
#[cfg(target_os = "android")]
use tauri::{plugin::PluginHandle, Manager};

#[cfg(target_os = "android")]
struct AndroidTransfers(PluginHandle<tauri::Wry>);

#[cfg(target_os = "android")]
static SERVICE_CLASS: std::sync::OnceLock<jni::objects::GlobalRef> = std::sync::OnceLock::new();
#[cfg(target_os = "android")]
static APPLICATION: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

pub fn attach_app(_app: &AppHandle) {
    #[cfg(target_os = "android")]
    {
        let _ = APPLICATION.set(_app.clone());
    }
}

#[cfg(target_os = "android")]
pub fn activity_ready(env: &mut jni::JNIEnv<'_>) {
    if SERVICE_CLASS.get().is_none() {
        if let Ok(class) = env.find_class("app/luminashelf/client/DownloadService") {
            if let Ok(reference) = env.new_global_ref(class) {
                let _ = SERVICE_CLASS.set(reference);
            }
        }
    }
    if let Some(app) = APPLICATION.get() {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if handle.webview_windows().is_empty() {
                if let Some(config) = handle.config().app.windows.first() {
                    if let Err(error) = tauri::WebviewWindowBuilder::from_config(&handle, config)
                        .and_then(|builder| builder.build())
                    {
                        eprintln!("restore Android activity window: {error}");
                    }
                }
            }
        });
    }
}

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
        // This route must work with no activity: Wry's mobile-plugin dispatcher
        // requires a live activity and panics after a Recents dismissal.
        let _ = app;
        if let Some(class) = SERVICE_CLASS.get() {
            let context = ndk_context::android_context();
            // The VM pointer belongs to Android and outlives this process.
            let result = (|| -> Result<(), jni::errors::Error> {
                let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }?;
                let mut env = vm.attach_current_thread()?;
                let id = env.new_string(&task.id)?;
                let title = env.new_string(&task.title)?;
                let class: &jni::objects::JClass<'_> = class.as_obj().into();
                env.call_static_method(
                    class,
                    "update",
                    "(Ljava/lang/String;Ljava/lang/String;IZ)V",
                    &[
                        (&id).into(),
                        (&title).into(),
                        progress.into(),
                        task.state.is_active().into(),
                    ],
                )?;
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("update Android download service: {error}");
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = (app, task);
}

pub async fn share(
    app: &AppHandle,
    path: &std::path::Path,
    mime: &str,
    view: bool,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        app.state::<AndroidTransfers>()
            .0
            .run_mobile_plugin_async::<serde_json::Value>(
                "share",
                serde_json::json!({ "path": path.to_string_lossy(), "mime": mime, "view": view }),
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, path, mime, view);
        Err("请使用导出电子书保存副本。".into())
    }
}
