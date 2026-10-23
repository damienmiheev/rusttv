//! Android entry point and the JNI [`Platform`] (ARCHITECTURE.md § Android bridge).
//!
//! Calls go to the Kotlin `ai.rusttv.launcher.Bridge` object in `android/`: one
//! coarse call per operation, plain types across the boundary. This is the only
//! module with `unsafe` (CLAUDE.md); each block has a `SAFETY:` comment.
#![allow(unsafe_code)]

use android_activity::{AndroidApp, MainEvent, PollEvent};
use jni::objects::{GlobalRef, JClass, JIntArray, JObject, JObjectArray, JString, JValue};
use jni::{JNIEnv, JavaVM};

use crate::model::App;
use crate::platform::{AppEntry, Artwork, Bitmap, Platform, PlatformError};
use crate::ui::{Lifecycle, RunOptions, run};

/// Size banners are drawn at: the Leanback banner size (320×180, 16:9).
const BANNER_SIZE: (i32, i32) = (320, 180);

/// Size square icons are drawn at, for apps without a banner.
const ICON_SIZE: i32 = 96;

const BRIDGE_CLASS: &str = "ai.rusttv.launcher.Bridge";

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("RustTV")
            .with_max_level(log::LevelFilter::Info),
    );

    let lifecycle = Lifecycle::default();
    let events = lifecycle.clone();
    let listener = move |event: &PollEvent<'_>| match event {
        PollEvent::Main(MainEvent::Pause) => events.paused(),
        PollEvent::Main(MainEvent::Resume { .. }) => events.resumed(),
        _ => {}
    };
    if let Err(error) = slint::android::init_with_event_listener(app.clone(), listener) {
        log::error!("starting the Slint Android backend failed: {error}");
        return;
    }
    let platform = match AndroidPlatform::new(&app) {
        Ok(platform) => platform,
        Err(error) => {
            log::error!("connecting to the Kotlin bridge failed: {error}");
            return;
        }
    };
    let options = RunOptions {
        right_to_left: false,
        simulated_pause: None,
        order_file: app.internal_data_path().map(|dir| dir.join("order.toml")),
        lifecycle,
    };
    if let Err(error) = run(platform, options) {
        log::error!("the home screen stopped: {error}");
    }
}

/// [`Platform`] backed by Android's package manager, through the Kotlin bridge.
struct AndroidPlatform {
    app: AndroidApp,
    vm: JavaVM,
    /// The `Bridge` class, loaded once through the app's class loader.
    bridge: GlobalRef,
}

impl std::fmt::Debug for AndroidPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AndroidPlatform").finish_non_exhaustive()
    }
}

impl AndroidPlatform {
    fn new(app: &AndroidApp) -> Result<Self, PlatformError> {
        // SAFETY: android-activity guarantees `vm_as_ptr` is the process's valid
        // `JavaVM*` for the lifetime of the app.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }
            .map_err(|error| PlatformError::call("JavaVM", error.to_string()))?;
        let mut env = vm
            .attach_current_thread_permanently()
            .map_err(|error| PlatformError::call("attach thread", error.to_string()))?;
        let activity = activity(app);
        // `FindClass` on a native thread only sees system classes, so the app's
        // own class is loaded through the activity's class loader.
        let bridge = java("load bridge", &mut env, |env| {
            let loader = env
                .call_method(
                    &activity,
                    "getClassLoader",
                    "()Ljava/lang/ClassLoader;",
                    &[],
                )?
                .l()?;
            let name = env.new_string(BRIDGE_CLASS)?;
            let class = env
                .call_method(
                    &loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            env.new_global_ref(class)
        })?;
        Ok(Self {
            app: app.clone(),
            vm,
            bridge,
        })
    }

    /// Runs `f` with a JNI env, the bridge class and the activity (a `Context`).
    fn with_bridge<T>(
        &self,
        operation: &'static str,
        f: impl FnOnce(&mut JNIEnv<'_>, &JClass<'_>, &JObject<'_>) -> jni::errors::Result<T>,
    ) -> Result<T, PlatformError> {
        let mut env = self
            .vm
            .attach_current_thread_permanently()
            .map_err(|error| PlatformError::call(operation, error.to_string()))?;
        let activity = activity(&self.app);
        // SAFETY: `bridge` is a live global reference to a `java.lang.Class`
        // (from `loadClass`), held by `self` for as long as this borrow.
        let bridge = unsafe { JClass::from_raw(self.bridge.as_obj().as_raw()) };
        java(operation, &mut env, |env| f(env, &bridge, &activity))
    }
}

impl Platform for AndroidPlatform {
    fn list_apps(&self) -> Result<Vec<AppEntry>, PlatformError> {
        self.with_bridge("list apps", |env, bridge, activity| {
            let array = env
                .call_static_method(
                    bridge,
                    "listApps",
                    "(Landroid/content/Context;)[Ljava/lang/String;",
                    &[JValue::Object(activity)],
                )?
                .l()?;
            let array = JObjectArray::from(array);
            let count = env.get_array_length(&array)?;
            let mut apps = Vec::new();
            for index in 0..count {
                let item = JString::from(env.get_object_array_element(&array, index)?);
                let text: String = env.get_string(&item)?.into();
                env.delete_local_ref(item)?;
                // "component\nlabel"; the bridge never puts a newline in a component.
                let Some((id, label)) = text.split_once('\n') else {
                    log::warn!("bridge returned a malformed app entry: {text:?}");
                    continue;
                };
                // Drawing banners takes ~7 ms per app: they load after the
                // first frame, through `artwork`.
                apps.push(AppEntry {
                    app: App {
                        id: id.to_owned(),
                        label: label.to_owned(),
                    },
                    artwork: Artwork::Later,
                });
            }
            Ok(apps)
        })
    }

    fn launch(&mut self, id: &str) -> Result<(), PlatformError> {
        let started = self.with_bridge("launch", |env, bridge, activity| {
            let component = env.new_string(id)?;
            env.call_static_method(
                bridge,
                "launch",
                "(Landroid/content/Context;Ljava/lang/String;)Z",
                &[JValue::Object(activity), JValue::Object(&component)],
            )?
            .z()
        })?;
        if started {
            Ok(())
        } else {
            Err(PlatformError::not_launchable(id))
        }
    }

    fn take_apps_changed(&mut self) -> Result<bool, PlatformError> {
        self.with_bridge("check app changes", |env, bridge, _activity| {
            env.call_static_method(bridge, "takeAppsChanged", "()Z", &[])?
                .z()
        })
    }

    fn artwork(&self, id: &str) -> Result<Artwork, PlatformError> {
        self.with_bridge("load artwork", |env, bridge, activity| {
            artwork(env, bridge, activity, id)
        })
    }

    fn artwork_loaded(&self) {
        let freed = self.with_bridge("free decoded artwork", |env, bridge, _activity| {
            env.call_static_method(bridge, "artworkLoaded", "()V", &[])?;
            Ok(())
        });
        if let Err(error) = freed {
            log::warn!("{error}");
        }
    }

    fn notices(&self) -> Result<String, PlatformError> {
        self.with_bridge("load notices", |env, bridge, activity| {
            let text = env
                .call_static_method(
                    bridge,
                    "notices",
                    "(Landroid/content/Context;)Ljava/lang/String;",
                    &[JValue::Object(activity)],
                )?
                .l()?;
            let text = JString::from(text);
            Ok(env.get_string(&text)?.into())
        })
    }
}

/// The app's banner, else its icon, else nothing.
fn artwork(
    env: &mut JNIEnv<'_>,
    bridge: &JClass<'_>,
    activity: &JObject<'_>,
    id: &str,
) -> jni::errors::Result<Artwork> {
    let component = env.new_string(id)?;
    let (width, height) = BANNER_SIZE;
    let banner = pixels(env, bridge, activity, &component, "banner", width, height)?;
    let artwork = match banner {
        Some(bitmap) => Artwork::Banner(bitmap),
        None => match pixels(
            env, bridge, activity, &component, "icon", ICON_SIZE, ICON_SIZE,
        )? {
            Some(bitmap) => Artwork::Icon(bitmap),
            None => Artwork::Missing,
        },
    };
    env.delete_local_ref(component)?;
    Ok(artwork)
}

/// Calls `Bridge.<method>(context, component, width, height): IntArray?` and
/// converts the ARGB pixels; `None` when the bridge returns null.
fn pixels(
    env: &mut JNIEnv<'_>,
    bridge: &JClass<'_>,
    activity: &JObject<'_>,
    component: &JString<'_>,
    method: &str,
    width: i32,
    height: i32,
) -> jni::errors::Result<Option<Bitmap>> {
    let array = env
        .call_static_method(
            bridge,
            method,
            "(Landroid/content/Context;Ljava/lang/String;II)[I",
            &[
                JValue::Object(activity),
                JValue::Object(component),
                JValue::Int(width),
                JValue::Int(height),
            ],
        )?
        .l()?;
    if array.is_null() {
        return Ok(None);
    }
    let array = JIntArray::from(array);
    // Both sizes are small positive constants.
    let mut argb = vec![0_i32; (width * height) as usize];
    env.get_int_array_region(&array, 0, &mut argb)?;
    env.delete_local_ref(array)?;
    let rgba = argb
        .iter()
        .flat_map(|value| {
            let [a, r, g, b] = value.to_be_bytes();
            [r, g, b, a]
        })
        .collect();
    Ok(Bitmap::new(width as u32, height as u32, rgba))
}

/// The activity, usable as an Android `Context`.
fn activity(app: &AndroidApp) -> JObject<'static> {
    // SAFETY: android-activity keeps a global reference to the activity for the
    // lifetime of the app; we only borrow it and never delete it.
    unsafe { JObject::from_raw(app.activity_as_ptr().cast()) }
}

/// Runs JNI calls in a local frame (freeing their local references), and turns a
/// pending Java exception into a [`PlatformError`] with its message.
fn java<T>(
    operation: &'static str,
    env: &mut JNIEnv<'_>,
    f: impl FnOnce(&mut JNIEnv<'_>) -> jni::errors::Result<T>,
) -> Result<T, PlatformError> {
    let result = env.with_local_frame(64, |env| f(env));
    result.map_err(|error| {
        let message = match error {
            jni::errors::Error::JavaException => java_exception(env),
            other => other.to_string(),
        };
        PlatformError::call(operation, message)
    })
}

/// Clears the pending Java exception and returns its `toString()`.
fn java_exception(env: &mut JNIEnv<'_>) -> String {
    let Ok(throwable) = env.exception_occurred() else {
        return "Java exception".to_owned();
    };
    // Must be cleared before any other JNI call.
    let _ = env.exception_clear();
    env.call_method(&throwable, "toString", "()Ljava/lang/String;", &[])
        .and_then(|value| value.l())
        .and_then(|text| env.get_string(&JString::from(text)).map(String::from))
        .unwrap_or_else(|_| "Java exception".to_owned())
}
