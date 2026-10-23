# Native memory profile: RustTV on the Philips 65OLED818/12, 2026-10-23

Perfetto heapprofd (sampling every 4 KiB) from cold start through 100 s idle,
RustTV at commit 80c4aca plus `<profileable android:shell="true"/>`, 31 apps.
PSS sampled every 10 s: 106 MB at 10 s (startup), then a flat 63.5 MB from
20 s to 100 s (native heap 30 MB). The occasional +30 MB idle jump seen
earlier did not happen in this run (nor in the three runs before it; it was
last seen once right after the backdrop bitmaps were removed).

Native memory still allocated at the end, by the first frame in our library:

| MB | What |
|---|---|
| 7.1 | Slint image buffers: the 31 banners, one copy each (320×180 RGBA) |
| 5.4 | Skia `SkData::NewWithCopy`: Slint's Skia renderer copies every image's pixels into Skia once (`i-slint-renderer-skia` `cached_image.rs`, all pixel formats) |
| 10.5 | No frame in our library (the stack ends in code without unwind info, most likely the Mali GPU driver) |
| ~4 | Fonts, Slint properties, GL, other small items |

So each banner costs ~460 KB (Slint copy + Skia copy). Avoiding the Skia copy
needs a change in Slint (sharing the buffer with `raster_from_data` instead of
`Data::new_copy`); worth raising upstream.
