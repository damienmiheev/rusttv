//! Phase 1b spike: can a Slint app be an Android TV launcher?
//!
//! Throwaway code to answer RISKS.md uncertainties 1–4; see README.md.
//! Quality bar: enough to measure, not to ship.

use std::rc::Rc;
use std::time::Instant;

use android_activity::AndroidApp;
use jni::JavaVM;
use jni::objects::{JClass, JIntArray, JObject, JObjectArray, JString, JValue};
use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

slint::slint! {
    export struct Tile { label: string, package: string, banner: image }

    export component Home inherits Window {
        in property <[Tile]> tiles;
        in property <string> status;
        in-out property <int> selected: 0;
        property <int> columns: 5;
        // Logical units: a 1080p TV is 960x540 at 2x density.
        property <length> tile-w: 156px;
        property <length> tile-h: 88px;
        property <length> gap: 14px;
        property <length> row-h: tile-h + 30px;
        callback activate(string);
        callback key-seen(string);
        background: #0e0f13;
        forward-focus: keys;

        keys := FocusScope {
            key-pressed(event) => {
                root.key-seen(event.text);
                if (event.text == Key.RightArrow) {
                    selected = min(selected + 1, tiles.length - 1);
                } else if (event.text == Key.LeftArrow) {
                    selected = max(selected - 1, 0);
                } else if (event.text == Key.DownArrow) {
                    selected = min(selected + columns, tiles.length - 1);
                } else if (event.text == Key.UpArrow) {
                    selected = max(selected - columns, 0);
                } else if (event.text == Key.Return || event.text == " ") {
                    root.activate(tiles[selected].package);
                } else if (event.text == "\u{f748}") {
                    // Android BACK. A launcher's home screen has nowhere to go
                    // back to, so consume it instead of letting Android close us.
                } else {
                    return reject;
                }
                accept
            }
        }

        Text {
            x: 56px; y: 32px;
            text: "RustTV spike";
            color: #f2f2f2; font-size: 22px; font-weight: 700;
        }
        Text {
            x: 56px; y: 62px;
            text: status;
            color: #9aa0aa; font-size: 12px;
        }

        Rectangle {
            x: 56px;
            // Keep the selected row on screen: scroll one row at a time.
            y: 100px - max(0, floor(selected / columns) - 1) * (row-h + gap);
            animate y { duration: 120ms; }

            for tile[i] in tiles: Rectangle {
                x: mod(i, columns) * (tile-w + gap);
                y: floor(i / columns) * (row-h + gap);
                width: tile-w; height: row-h;

                Rectangle {
                    y: 0; width: tile-w; height: tile-h;
                    border-radius: 6px;
                    clip: true;
                    background: #22252d;
                    border-width: i == selected ? 2px : 0px;
                    border-color: #f5a524;
                    Image {
                        source: tile.banner;
                        width: parent.width; height: parent.height;
                        image-fit: contain;
                    }
                }
                Text {
                    y: tile-h + 6px; height: 16px;
                    width: tile-w;
                    vertical-alignment: top;
                    text: tile.label;
                    overflow: elide;
                    color: i == selected ? #ffffff : #9aa0aa;
                    font-size: 11px;
                }
            }
        }
    }
}

const BANNER_W: i32 = 320;
const BANNER_H: i32 = 180;

struct AppEntry {
    label: String,
    package: String,
    banner: Image,
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("RustTVSpike")
            .with_max_level(log::LevelFilter::Info),
    );
    let started = Instant::now();
    log::info!("android_main entered");

    if let Err(error) = slint::android::init(app.clone()) {
        log::error!("slint::android::init failed: {error}");
        return;
    }
    let home = match Home::new() {
        Ok(home) => home,
        Err(error) => {
            log::error!("creating the window failed: {error}");
            return;
        }
    };
    log::info!("window created after {:?}", started.elapsed());

    let apps = match list_apps(&app) {
        Ok(apps) => apps,
        Err(error) => {
            log::error!("listing apps over JNI failed: {error}");
            Vec::new()
        }
    };
    log::info!("listed {} apps with banners after {:?}", apps.len(), started.elapsed());

    home.set_status(format!("{} apps · D-pad to move, OK to launch", apps.len()).into());
    let tiles: Vec<Tile> = apps
        .into_iter()
        .map(|entry| Tile {
            label: entry.label.into(),
            package: entry.package.into(),
            banner: entry.banner,
        })
        .collect();
    home.set_tiles(ModelRc::from(Rc::new(VecModel::from(tiles))));

    home.on_key_seen(|text| {
        let codes: Vec<u32> = text.chars().map(u32::from).collect();
        log::info!("key-pressed text={text:?} codepoints={codes:x?}");
    });
    let launcher_app = app.clone();
    home.on_activate(move |package| {
        let launched = launch(&launcher_app, &package);
        log::info!("launch {package}: {launched:?}");
    });

    log::info!("entering event loop after {:?}", started.elapsed());
    if let Err(error) = home.run() {
        log::error!("event loop failed: {error}");
    }
}

/// Runs `f` with a JNI env and the `ai.rusttv.spike.Bridge` class.
///
/// `FindClass` from a native thread only sees system classes, so the app's
/// own class is loaded through the activity's class loader.
fn with_bridge<T>(
    app: &AndroidApp,
    f: impl FnOnce(&mut jni::JNIEnv, &JClass, &JObject) -> jni::errors::Result<T>,
) -> jni::errors::Result<T> {
    // SAFETY: android-activity guarantees both pointers are valid for the
    // lifetime of the app: the process's JavaVM and a global reference to
    // the Activity.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let mut env = vm.attach_current_thread()?;
    let loader = env
        .call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let name = env.new_string("ai.rusttv.spike.Bridge")?;
    let class = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    let class = JClass::from(class);
    f(&mut env, &class, &activity)
}

fn list_apps(app: &AndroidApp) -> jni::errors::Result<Vec<AppEntry>> {
    with_bridge(app, |env, bridge, activity| {
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
            let Some((label, package)) = text.split_once('\n') else {
                continue;
            };
            let package_java = env.new_string(package)?;
            let pixels = env
                .call_static_method(
                    bridge,
                    "banner",
                    "(Landroid/content/Context;Ljava/lang/String;II)[I",
                    &[
                        JValue::Object(activity),
                        JValue::Object(&package_java),
                        JValue::Int(BANNER_W),
                        JValue::Int(BANNER_H),
                    ],
                )?
                .l()?;
            let pixels = JIntArray::from(pixels);
            let mut argb = vec![0_i32; (BANNER_W * BANNER_H) as usize];
            env.get_int_array_region(&pixels, 0, &mut argb)?;
            apps.push(AppEntry {
                label: label.to_owned(),
                package: package.to_owned(),
                banner: argb_to_image(&argb),
            });
            // Avoid running out of JNI local references in the loop.
            env.delete_local_ref(item)?;
            env.delete_local_ref(package_java)?;
            env.delete_local_ref(pixels)?;
        }
        Ok(apps)
    })
}

fn argb_to_image(argb: &[i32]) -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(BANNER_W as u32, BANNER_H as u32);
    for (pixel, &value) in buffer.make_mut_slice().iter_mut().zip(argb) {
        let [a, r, g, b] = value.to_be_bytes();
        *pixel = Rgba8Pixel { r, g, b, a };
    }
    Image::from_rgba8(buffer)
}

fn launch(app: &AndroidApp, package: &str) -> jni::errors::Result<bool> {
    with_bridge(app, |env, bridge, activity| {
        let package = env.new_string(package)?;
        env.call_static_method(
            bridge,
            "launch",
            "(Landroid/content/Context;Ljava/lang/String;)Z",
            &[JValue::Object(activity), JValue::Object(&package)],
        )?
        .z()
    })
}
