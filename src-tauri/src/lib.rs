use lumina_core::{AppResolver, GutendexProvider, ProviderDescriptor, ProviderRegistry};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreStatus {
    name: &'static str,
    version: &'static str,
    rust_core: bool,
    network_stack: &'static str,
}

#[tauri::command]
fn core_status() -> CoreStatus {
    CoreStatus {
        name: "LuminaShelf",
        version: env!("CARGO_PKG_VERSION"),
        rust_core: true,
        network_stack: "Tokio · Reqwest · Hickory",
    }
}

#[tauri::command]
fn provider_descriptors() -> Result<Vec<ProviderDescriptor>, String> {
    let resolver = AppResolver::system().map_err(|error| error.to_string())?;
    let registry = ProviderRegistry::default();
    let gutendex = GutendexProvider::new(resolver).map_err(|error| error.to_string())?;
    registry.register(gutendex);
    Ok(registry.descriptors())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![core_status, provider_descriptors])
        .run(tauri::generate_context!())
        .expect("failed to run LuminaShelf");
}
