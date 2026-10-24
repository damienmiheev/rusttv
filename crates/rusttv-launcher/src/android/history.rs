//! The history on the TV (HISTORY.md): the [`HistoryPlatform`] over JNI, and
//! the native entry points of the door (`History.native*` in Kotlin, called
//! by `HistoryProvider` on its binder threads).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue};
use jni::sys::{jbyteArray, jstring};
use jni::{JNIEnv, JavaVM};
use rusttv_history::door::{self, CommitKind, CommitResult};
use rusttv_history::tv::{TvClock, TvDevice};
use rusttv_history::{HistoryError, HistoryId, Launcher, SETTING_KEYS};

use crate::history::{History, HistoryPlatform};
use crate::order_file;

/// The VM, the application context and the Kotlin `History` class.
struct Jni {
    vm: JavaVM,
    context: GlobalRef,
    class: GlobalRef,
}

impl Jni {
    /// Runs `f` on this thread (attached once, permanently) in a local frame.
    fn call<T>(
        &self,
        f: impl FnOnce(&mut JNIEnv<'_>, &JClass<'_>, &JObject<'_>) -> jni::errors::Result<T>,
    ) -> Result<T, HistoryError> {
        let mut env = self
            .vm
            .attach_current_thread_permanently()
            .map_err(|error| HistoryError::Storage(error.to_string()))?;
        // SAFETY: `class` is a live global reference to `java.lang.Class`
        // (the History class), held by `self` for as long as this borrow.
        let class = unsafe { JClass::from_raw(self.class.as_obj().as_raw()) };
        let context = self.context.as_obj();
        env.with_local_frame(16, |env| f(env, &class, context))
            .map_err(|error| {
                let _ = env.exception_clear();
                HistoryError::Storage(error.to_string())
            })
    }

    fn files_dir(&self) -> Result<PathBuf, HistoryError> {
        self.call(|env, class, context| {
            let dir = env
                .call_static_method(
                    class,
                    "filesDir",
                    "(Landroid/content/Context;)Ljava/lang/String;",
                    &[JValue::Object(context)],
                )?
                .l()?;
            Ok(PathBuf::from(String::from(
                env.get_string(&JString::from(dir))?,
            )))
        })
    }
}

struct Device {
    jni: Arc<Jni>,
    files: PathBuf,
}

/// The index of an allowlisted key, as `SecureSettings` numbers them.
fn key_index(key: &str) -> Result<i32, HistoryError> {
    SETTING_KEYS
        .iter()
        .position(|known| *known == key)
        .and_then(|index| i32::try_from(index).ok())
        .ok_or_else(|| HistoryError::Invalid(format!("{key} is not allowed")))
}

impl TvDevice for Device {
    fn launcher(&self) -> Result<Launcher, HistoryError> {
        Ok(order_file::launcher_state(&self.files))
    }

    fn write_launcher(&mut self, launcher: &Launcher) -> Result<(), HistoryError> {
        order_file::write_launcher_state(&self.files, launcher)
            .map_err(|error| HistoryError::Storage(error.to_string()))
    }

    fn setting(&self, key: &str) -> Result<Option<String>, HistoryError> {
        let index = key_index(key)?;
        self.jni.call(|env, class, context| {
            let value = env
                .call_static_method(
                    class,
                    "setting",
                    "(Landroid/content/Context;I)Ljava/lang/String;",
                    &[JValue::Object(context), JValue::Int(index)],
                )?
                .l()?;
            if value.is_null() {
                return Ok(None);
            }
            Ok(Some(String::from(env.get_string(&JString::from(value))?)))
        })
    }

    fn can_write_settings(&self) -> bool {
        self.jni
            .call(|env, class, context| {
                env.call_static_method(
                    class,
                    "canWriteSettings",
                    "(Landroid/content/Context;)Z",
                    &[JValue::Object(context)],
                )?
                .z()
            })
            .unwrap_or(false)
    }

    fn put_setting(&mut self, key: &str, value: &str) -> Result<(), HistoryError> {
        let index = key_index(key)?;
        let written = self.jni.call(|env, class, context| {
            let value = env.new_string(value)?;
            env.call_static_method(
                class,
                "putSetting",
                "(Landroid/content/Context;ILjava/lang/String;)Z",
                &[
                    JValue::Object(context),
                    JValue::Int(index),
                    JValue::Object(&value),
                ],
            )?
            .z()
        })?;
        if written {
            Ok(())
        } else {
            Err(HistoryError::Storage(format!(
                "Android refused to set {key}"
            )))
        }
    }
}

struct Clock {
    jni: Arc<Jni>,
}

impl TvClock for Clock {
    fn elapsed_ms(&self) -> u64 {
        self.jni
            .call(|env, class, _| env.call_static_method(class, "elapsedMs", "()J", &[])?.j())
            .map_or(0, |ms| u64::try_from(ms).unwrap_or(0))
    }

    fn boot_count(&self) -> u64 {
        self.jni
            .call(|env, class, context| {
                env.call_static_method(
                    class,
                    "bootCount",
                    "(Landroid/content/Context;)J",
                    &[JValue::Object(context)],
                )?
                .j()
            })
            .map_or(0, |count| u64::try_from(count).unwrap_or(0))
    }

    fn now_unix(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            })
    }
}

struct Platform {
    jni: Arc<Jni>,
    device: Device,
    clock: Clock,
}

impl HistoryPlatform for Platform {
    fn root(&self) -> PathBuf {
        self.device.files.clone()
    }

    fn parts(&mut self) -> (&mut dyn TvDevice, &dyn TvClock) {
        (&mut self.device, &self.clock)
    }

    fn new_id(&self) -> HistoryId {
        let bytes = self
            .jni
            .call(|env, class, _| {
                let array = env
                    .call_static_method(class, "randomBytes", "(I)[B", &[JValue::Int(16)])?
                    .l()?;
                env.convert_byte_array(JByteArray::from(array))
            })
            .unwrap_or_default();
        let mut id = [0; 16];
        for (slot, byte) in id.iter_mut().zip(bytes) {
            *slot = byte;
        }
        HistoryId::from_random(id)
    }

    fn test_build(&self) -> bool {
        // Unknown counts as a test build: approvals stay off.
        self.jni
            .call(|env, class, context| {
                env.call_static_method(
                    class,
                    "testBuild",
                    "(Landroid/content/Context;)Z",
                    &[JValue::Object(context)],
                )?
                .z()
            })
            .unwrap_or(true)
    }

    fn lower_priority(&self) {
        if let Err(error) = self.jni.call(|env, class, _| {
            env.call_static_method(class, "lowerPriority", "()V", &[])
                .map(|_| ())
        }) {
            log::warn!("history: lowering its priority failed: {error}");
        }
    }
}

/// Builds the platform from a JNI env, the `History` class and a context.
fn platform(
    env: &mut JNIEnv<'_>,
    class: &JClass<'_>,
    context: &JObject<'_>,
) -> Result<Platform, HistoryError> {
    let storage = |error: jni::errors::Error| HistoryError::Storage(error.to_string());
    let vm = env.get_java_vm().map_err(storage)?;
    let app = env
        .call_method(
            context,
            "getApplicationContext",
            "()Landroid/content/Context;",
            &[],
        )
        .and_then(|value| value.l())
        .map_err(storage)?;
    let jni = Arc::new(Jni {
        vm,
        context: env.new_global_ref(app).map_err(storage)?,
        class: env.new_global_ref(class).map_err(storage)?,
    });
    let files = jni.files_dir()?;
    Ok(Platform {
        device: Device {
            jni: Arc::clone(&jni),
            files,
        },
        clock: Clock {
            jni: Arc::clone(&jni),
        },
        jni,
    })
}

/// Starts the process's history thread from the activity (android_main),
/// loading the Kotlin class through the activity's class loader.
pub(super) fn start_from_activity(vm: &JavaVM, activity: &JObject<'_>) {
    let started = (|| -> Result<(), HistoryError> {
        let mut env = vm
            .attach_current_thread_permanently()
            .map_err(|error| HistoryError::Storage(error.to_string()))?;
        let class = super::load_class(&mut env, activity, "ai.rusttv.launcher.History")
            .map_err(|error| HistoryError::Storage(error.to_string()))?;
        let class = JClass::from(class);
        let platform = platform(&mut env, &class, activity)?;
        History::global(|| Box::new(platform));
        Ok(())
    })();
    if let Err(error) = started {
        log::warn!("history is off: {error}");
    }
}

/// The history thread for a door call; starts it if the door started the process.
fn history_for(
    env: &mut JNIEnv<'_>,
    class: &JClass<'_>,
    context: &JObject<'_>,
) -> Option<&'static History> {
    if let Some(history) = History::get() {
        return Some(history);
    }
    match platform(env, class, context) {
        Ok(platform) => Some(History::global(|| Box::new(platform))),
        Err(error) => {
            log::warn!("history is off: {error}");
            None
        }
    }
}

fn byte_array(env: &mut JNIEnv<'_>, bytes: &[u8]) -> jbyteArray {
    env.byte_array_from_slice(bytes)
        .map_or(std::ptr::null_mut(), |array| array.into_raw())
}

/// `History.nativeExport(context, since)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_rusttv_launcher_History_nativeExport<'local>(
    mut env: JNIEnv<'local>,
    class: JClass<'local>,
    context: JObject<'local>,
    since: JString<'local>,
) -> jbyteArray {
    let since = if since.is_null() {
        None
    } else {
        env.get_string(&since).ok().map(String::from)
    };
    let answer = match history_for(&mut env, &class, &context) {
        Some(history) => history
            .export(since)
            .unwrap_or_else(|refused| door::read_error(refused.code)),
        None => door::read_error("HISTORY_STORAGE"),
    };
    byte_array(&mut env, &answer)
}

/// `History.nativePending(context)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_rusttv_launcher_History_nativePending<'local>(
    mut env: JNIEnv<'local>,
    class: JClass<'local>,
    context: JObject<'local>,
) -> jbyteArray {
    let answer = match history_for(&mut env, &class, &context) {
        Some(history) => history
            .pending()
            .map_or_else(|refused| door::read_error(refused.code), String::into_bytes),
        None => door::read_error("HISTORY_STORAGE"),
    };
    byte_array(&mut env, &answer)
}

/// `History.nativeCommit(context, arg, bytes)`: the result line.
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_rusttv_launcher_History_nativeCommit<'local>(
    mut env: JNIEnv<'local>,
    class: JClass<'local>,
    context: JObject<'local>,
    arg: JString<'local>,
    bytes: JByteArray<'local>,
) -> jstring {
    let failed = |code: &str| CommitResult {
        error: Some(code.to_owned()),
        head: None,
    };
    let result = (|| {
        let arg = String::from(env.get_string(&arg).ok()?);
        let bytes = env.convert_byte_array(&bytes).ok()?;
        let history = history_for(&mut env, &class, &context)?;
        Some(match CommitKind::parse_arg(&arg) {
            Ok((kind, _)) => history.commit(kind, bytes),
            Err(_) => failed("HISTORY_INVALID"),
        })
    })()
    .unwrap_or_else(|| failed("HISTORY_STORAGE"));
    env.new_string(result.encode())
        .map_or(std::ptr::null_mut(), |text| text.into_raw())
}
