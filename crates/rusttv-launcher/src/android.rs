//! Android entry point and the JNI [`Platform`] (ARCHITECTURE.md § Android bridge).
//!
//! Calls go to the Kotlin `ai.rusttv.launcher.Bridge` object in `android/`: one
//! coarse call per operation, plain types across the boundary. This is the only
//! module with `unsafe` (CLAUDE.md); each block has a `SAFETY:` comment.
#![allow(unsafe_code)]

use android_activity::{AndroidApp, MainEvent, PollEvent};
use jni::objects::{
    GlobalRef, JByteArray, JClass, JIntArray, JObject, JObjectArray, JString, JValue,
};
use jni::{JNIEnv, JavaVM};

mod history;

use rusttv_remote::protocol::{Key, PictureKind, PointerAction, TextEdit, TextField};

use crate::model::{App, Category};
use crate::platform::{
    AppEntry, Artwork, Bitmap, Pictures, Platform, PlatformError, PointerState, UpdateStatus,
};
use crate::ui::{Lifecycle, RunOptions, run};

/// Size banners are drawn at: the Leanback banner size (320×180, 16:9).
const BANNER_SIZE: (i32, i32) = (320, 180);

/// Size square icons are drawn at, for apps without a banner.
const ICON_SIZE: i32 = 96;

/// Size icons are drawn at for the phones' tiles (shown at about 64 px on a
/// 3× phone screen).
const PICTURE_ICON_SIZE: i32 = 192;

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
    history::start_from_activity(&platform.vm, &activity(&app));
    let language = platform.language();
    let options = RunOptions {
        language,
        right_to_left: false,
        simulated_pause: None,
        order_file: app.internal_data_path().map(|dir| dir.join("order.toml")),
        lifecycle,
        remote_pairing: false,
        remote_port: None,
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
        let bridge = java("load bridge", &mut env, |env| {
            let class = load_class(env, &activity, BRIDGE_CLASS)?;
            env.new_global_ref(class)
        })?;
        Ok(Self {
            app: app.clone(),
            vm,
            bridge,
        })
    }

    /// The app's language as a BCP 47 tag (`Bridge.language`), or `None` if
    /// the call fails (Slint then picks from the system locale).
    fn language(&self) -> Option<String> {
        self.with_bridge("read language", |env, bridge, activity| {
            let tag = env
                .call_static_method(
                    bridge,
                    "language",
                    "(Landroid/content/Context;)Ljava/lang/String;",
                    &[JValue::Object(activity)],
                )?
                .l()?;
            let tag = JString::from(tag);
            Ok(String::from(env.get_string(&tag)?))
        })
        .inspect_err(|error| log::warn!("reading the language failed: {error}"))
        .ok()
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

    /// A built-in tile's banner as WebP, through `Bridge.webp`.
    fn builtin_picture(&self, width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
        let argb: Vec<i32> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[r, g, b, a]| i32::from_be_bytes([a, r, g, b]))
            .collect();
        let (width, height) = (i32::try_from(width).ok()?, i32::try_from(height).ok()?);
        self.with_bridge("built-in tile picture", |env, bridge, _activity| {
            let pixels = env.new_int_array(width * height)?;
            env.set_int_array_region(&pixels, 0, &argb)?;
            let array = env
                .call_static_method(
                    bridge,
                    "webp",
                    "([III)[B",
                    &[
                        JValue::Object(&pixels),
                        JValue::Int(width),
                        JValue::Int(height),
                    ],
                )?
                .l()?;
            env.convert_byte_array(JByteArray::from(array))
        })
        .map_err(|error| log::warn!("{error}"))
        .ok()
    }

    /// `Bridge.tilePicture`: the app's banner, else its icon, as WebP; a
    /// built-in tile's banner.
    fn picture(&self, id: &str) -> Option<(PictureKind, Vec<u8>)> {
        if let Some((width, height, rgba)) = crate::ui::builtin_banner(id) {
            return Some((
                PictureKind::Banner,
                self.builtin_picture(width, height, rgba)?,
            ));
        }
        let bytes = self
            .with_bridge("tile picture", |env, bridge, activity| {
                let component = env.new_string(id)?;
                let (width, height) = BANNER_SIZE;
                let array = env
                    .call_static_method(
                        bridge,
                        "tilePicture",
                        "(Landroid/content/Context;Ljava/lang/String;III)[B",
                        &[
                            JValue::Object(activity),
                            JValue::Object(&component),
                            JValue::Int(width),
                            JValue::Int(height),
                            JValue::Int(PICTURE_ICON_SIZE),
                        ],
                    )?
                    .l()?;
                if array.is_null() {
                    return Ok(None);
                }
                env.convert_byte_array(JByteArray::from(array)).map(Some)
            })
            .unwrap_or_else(|error| {
                log::warn!("{error}");
                None
            })?;
        let (&kind, webp) = bytes.split_first()?;
        let kind = if kind == 0 {
            PictureKind::Banner
        } else {
            PictureKind::Icon
        };
        Some((kind, webp.to_vec()))
    }

    /// Calls `Bridge.<method>(context): Unit`.
    fn call_with_context(
        &self,
        operation: &'static str,
        method: &'static str,
    ) -> Result<(), PlatformError> {
        self.with_bridge(operation, |env, bridge, activity| {
            env.call_static_method(
                bridge,
                method,
                "(Landroid/content/Context;)V",
                &[JValue::Object(activity)],
            )?;
            Ok(())
        })
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
                // "component\nlabel\ncategory"; the bridge never puts a
                // newline in a component or a label.
                let mut parts = text.splitn(3, '\n');
                let (Some(id), Some(label)) = (parts.next(), parts.next()) else {
                    log::warn!("bridge returned a malformed app entry: {text:?}");
                    continue;
                };
                let category = parts
                    .next()
                    .and_then(|value| value.parse().ok())
                    .map_or(Category::Other, Category::from_android);
                // Drawing banners takes ~7 ms per app: they load after the
                // first frame, through `artwork`.
                apps.push(AppEntry {
                    app: App {
                        id: id.to_owned(),
                        label: label.to_owned(),
                        category,
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

    fn update_status(&self) -> Result<UpdateStatus, PlatformError> {
        self.with_bridge("read update status", |env, bridge, activity| {
            let text = env
                .call_static_method(
                    bridge,
                    "updateStatus",
                    "(Landroid/content/Context;)Ljava/lang/String;",
                    &[JValue::Object(activity)],
                )?
                .l()?;
            let text: String = env.get_string(&JString::from(text))?.into();
            Ok(UpdateStatus::from_bridge(&text))
        })
    }

    fn check_for_update(&mut self) -> Result<(), PlatformError> {
        self.call_with_context("check for update", "checkForUpdate")
    }

    fn install_update(&mut self) -> Result<(), PlatformError> {
        self.call_with_context("install update", "installUpdate")
    }

    fn set_secure(&mut self, on: bool) {
        let set = self.with_bridge("secure window", |env, bridge, activity| {
            env.call_static_method(
                bridge,
                "setSecure",
                "(Landroid/app/Activity;Z)V",
                &[JValue::Object(activity), JValue::Bool(u8::from(on))],
            )?;
            Ok(())
        });
        if let Err(error) = set {
            log::warn!("{error}");
        }
    }

    fn random_code(&mut self) -> [u32; 4] {
        let code = self.with_bridge("random code", |env, bridge, _activity| {
            let array = JIntArray::from(
                env.call_static_method(bridge, "randomCode", "()[I", &[])?
                    .l()?,
            );
            let mut values = [0_i32; 4];
            env.get_int_array_region(&array, 0, &mut values)?;
            Ok(values.map(|value| u32::try_from(value).unwrap_or(0)))
        });
        // Without the OS source there is no safe code: an all-up code is still
        // valid input, and the failure is logged.
        code.unwrap_or_else(|error| {
            log::warn!("{error}");
            [0; 4]
        })
    }

    fn pictures(&self) -> Option<Pictures> {
        // SAFETY: the same process-wide `JavaVM*` as `self.vm`, valid for the
        // lifetime of the app.
        let vm = unsafe { JavaVM::from_raw(self.vm.get_java_vm_pointer()) }.ok()?;
        let bridge = std::sync::Arc::new(AndroidPlatform {
            app: self.app.clone(),
            vm,
            bridge: self.bridge.clone(),
        });
        let freeing = std::sync::Arc::clone(&bridge);
        Some(Pictures {
            draw: Box::new(move |id: &str| bridge.picture(id)),
            done: Box::new(move || freeing.artwork_loaded()),
        })
    }

    fn utc_offset_secs(&self) -> i32 {
        self.with_bridge("time zone", |env, bridge, _activity| {
            env.call_static_method(bridge, "utcOffsetSecs", "()I", &[])?
                .i()
        })
        .unwrap_or(0)
    }

    fn pointer_state(&self) -> PointerState {
        self.with_bridge("pointer state", |env, bridge, _activity| {
            let text = env
                .call_static_method(bridge, "pointerState", "()Ljava/lang/String;", &[])?
                .l()?;
            let text: String = env.get_string(&JString::from(text))?.into();
            Ok(PointerState::from_bridge(&text))
        })
        .unwrap_or_else(|error| {
            log::warn!("{error}");
            PointerState::Unavailable
        })
    }

    fn front_app(&self) -> Option<String> {
        self.with_bridge("front app", |env, bridge, _activity| {
            let text = env
                .call_static_method(bridge, "pointerApp", "()Ljava/lang/String;", &[])?
                .l()?;
            let text: String = env.get_string(&JString::from(text))?.into();
            Ok(text)
        })
        .inspect_err(|error| log::warn!("{error}"))
        .ok()
        .filter(|app| !app.is_empty())
    }

    fn front_field(&self) -> Option<TextField> {
        let text = self
            .with_bridge("front field", |env, bridge, _activity| {
                let text = env
                    .call_static_method(bridge, "pointerField", "()Ljava/lang/String;", &[])?
                    .l()?;
                let text: String = env.get_string(&JString::from(text))?.into();
                Ok(text)
            })
            .inspect_err(|error| log::warn!("{error}"))
            .ok()?;
        let (label, value) = text.split_once('\n')?;
        Some(TextField {
            label: label.to_owned(),
            value: value.to_owned(),
        })
    }

    fn pointer_text(&mut self, edit: TextEdit) {
        let (op, text, count) = match edit {
            TextEdit::Insert { text } => (0, text, 0),
            TextEdit::Delete { count } => (1, String::new(), i32::from(count)),
            TextEdit::Submit => (2, String::new(), 0),
        };
        let typed = self.with_bridge("pointer typing", |env, bridge, _activity| {
            let text = env.new_string(text)?;
            env.call_static_method(
                bridge,
                "pointerType",
                "(ILjava/lang/String;I)V",
                &[JValue::Int(op), JValue::Object(&text), JValue::Int(count)],
            )?;
            Ok(())
        });
        if let Err(error) = typed {
            log::warn!("{error}");
        }
    }

    fn hide_tv_keyboard(&mut self, hide: bool) {
        let done = self.with_bridge("tv keyboard", |env, bridge, _activity| {
            env.call_static_method(
                bridge,
                "hideKeyboard",
                "(Z)V",
                &[JValue::Bool(u8::from(hide))],
            )?;
            Ok(())
        });
        if let Err(error) = done {
            log::warn!("{error}");
        }
    }

    fn can_search(&self, app: &str) -> bool {
        self.with_bridge("can search", |env, bridge, activity| {
            let app = env.new_string(app)?;
            env.call_static_method(
                bridge,
                "canSearch",
                "(Landroid/content/Context;Ljava/lang/String;)Z",
                &[JValue::Object(activity), JValue::Object(&app)],
            )?
            .z()
        })
        .inspect_err(|error| log::warn!("{error}"))
        .unwrap_or(false)
    }

    fn search(&mut self, app: &str, query: &str) {
        let searched = self.with_bridge("search", |env, bridge, activity| {
            let app = env.new_string(app)?;
            let query = env.new_string(query)?;
            env.call_static_method(
                bridge,
                "search",
                "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V",
                &[
                    JValue::Object(activity),
                    JValue::Object(&app),
                    JValue::Object(&query),
                ],
            )?;
            Ok(())
        });
        if let Err(error) = searched {
            log::warn!("{error}");
        }
    }

    fn front_app_dpad(&self) -> bool {
        self.with_bridge("front app d-pad", |env, bridge, _activity| {
            env.call_static_method(bridge, "pointerDpad", "()Z", &[])?
                .z()
        })
        .inspect_err(|error| log::warn!("{error}"))
        .unwrap_or(false)
    }

    fn pointer(&mut self, dx: i16, dy: i16, action: PointerAction) {
        let action = match action {
            PointerAction::Move => 0,
            PointerAction::Click => 1,
            PointerAction::Scroll => 2,
        };
        let sent = self.with_bridge("pointer", |env, bridge, _activity| {
            env.call_static_method(
                bridge,
                "pointer",
                "(III)V",
                &[
                    JValue::Int(i32::from(dx)),
                    JValue::Int(i32::from(dy)),
                    JValue::Int(action),
                ],
            )?;
            Ok(())
        });
        if let Err(error) = sent {
            log::warn!("{error}");
        }
    }

    fn pointer_stop(&mut self) {
        let stopped = self.with_bridge("pointer stop", |env, bridge, _activity| {
            env.call_static_method(bridge, "pointerStop", "()V", &[])?;
            Ok(())
        });
        if let Err(error) = stopped {
            log::warn!("{error}");
        }
    }

    fn pointer_key(&mut self, key: Key) {
        // PointerService.key's codes.
        let code = match key {
            Key::Up => 0,
            Key::Down => 1,
            Key::Left => 2,
            Key::Right => 3,
            Key::Ok => 4,
            Key::Back => 5,
            Key::Home => 6,
            Key::PlayPause => 7,
            Key::Rewind => 8,
            Key::FastForward => 9,
            Key::VolumeUp => 10,
            Key::VolumeDown => 11,
            Key::Mute => 12,
            _ => return,
        };
        let pressed = self.with_bridge("pointer key", |env, bridge, _activity| {
            env.call_static_method(bridge, "pointerKey", "(I)V", &[JValue::Int(code)])?;
            Ok(())
        });
        if let Err(error) = pressed {
            log::warn!("{error}");
        }
    }

    fn open_pointer_settings(&mut self) {
        let opened = self.with_bridge("accessibility settings", |env, bridge, activity| {
            env.call_static_method(
                bridge,
                "openAccessibilitySettings",
                "(Landroid/content/Context;)V",
                &[JValue::Object(activity)],
            )?;
            Ok(())
        });
        if let Err(error) = opened {
            log::warn!("{error}");
        }
    }
}

/// `PointerService.nativeChanged()`: the pointer's service went on or off;
/// phones and the RustTV Remote page hear of it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_rusttv_launcher_PointerService_nativeChanged<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    crate::ui::pointer_changed();
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

/// Loads one of the app's own classes. `FindClass` on a native thread only
/// sees system classes, so it goes through the activity's class loader.
fn load_class<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject<'_>,
    name: &str,
) -> jni::errors::Result<JObject<'local>> {
    let loader = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let name = env.new_string(name)?;
    env.call_method(
        &loader,
        "loadClass",
        "(Ljava/lang/String;)Ljava/lang/Class;",
        &[JValue::Object(&name)],
    )?
    .l()
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
