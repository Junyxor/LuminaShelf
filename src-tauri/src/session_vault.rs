use lumina_core::ZLibrarySession;

#[cfg(target_os = "android")]
const SERVICE: &str = "app.luminashelf.client";
#[cfg(target_os = "android")]
const ZLIBRARY_SESSION_USER: &str = "zlibrary-eapi-session-v1";

pub fn supported() -> bool {
    cfg!(target_os = "android")
}

#[cfg(target_os = "android")]
fn prepare_store() -> Result<(), String> {
    keyring::use_native_store(false)
        .map_err(|error| format!("initialize Android secure store: {error}"))
}

#[cfg(target_os = "android")]
fn entry() -> Result<keyring_core::Entry, String> {
    prepare_store()?;
    keyring_core::Entry::new(SERVICE, ZLIBRARY_SESSION_USER)
        .map_err(|error| format!("open secure session entry: {error}"))
}

pub fn save(session: &ZLibrarySession) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        let bytes = serde_json::to_vec(session)
            .map_err(|error| format!("encode Z-Library session: {error}"))?;
        entry()?
            .set_secret(&bytes)
            .map_err(|error| format!("save Z-Library session to Android Keystore: {error}"))?;
        return Ok(true);
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = session;
        Ok(false)
    }
}

pub fn load() -> Result<Option<ZLibrarySession>, String> {
    #[cfg(target_os = "android")]
    {
        let entry = entry()?;
        let bytes = match entry.get_secret() {
            Ok(bytes) => bytes,
            Err(keyring_core::Error::NoEntry) => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "read Z-Library session from Android Keystore: {error}"
                ))
            }
        };
        let session = serde_json::from_slice::<ZLibrarySession>(&bytes)
            .map_err(|error| format!("decode stored Z-Library session: {error}"))?;
        if session.user_id.trim().is_empty() || session.user_key.trim().is_empty() {
            return Err("stored Z-Library session is empty".to_string());
        }
        return Ok(Some(session));
    }

    #[cfg(not(target_os = "android"))]
    {
        Ok(None)
    }
}

pub fn clear() -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        let entry = entry()?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(true),
            Err(error) => Err(format!(
                "delete Z-Library session from Android Keystore: {error}"
            )),
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        Ok(false)
    }
}

#[cfg(target_os = "android")]
#[allow(non_snake_case)]
#[no_mangle]
pub extern "system" fn Java_app_luminashelf_client_MainActivity_initNdkContext(
    env: jni::JNIEnv,
    _class: jni::objects::JObject,
    context: jni::objects::JObject,
) {
    use jni::objects::GlobalRef;
    use std::{ffi::c_void, sync::OnceLock};

    static CONTEXT: OnceLock<Option<GlobalRef>> = OnceLock::new();
    CONTEXT.get_or_init(|| match env.new_global_ref(&context) {
        Ok(reference) => {
            let vm = match env.get_java_vm() {
                Ok(vm) => vm,
                Err(_) => return None,
            };
            let vm = vm.get_java_vm_pointer() as *mut c_void;
            unsafe {
                ndk_context::initialize_android_context(vm, reference.as_obj().as_raw() as _);
            }
            Some(reference)
        }
        Err(_) => None,
    });
}
