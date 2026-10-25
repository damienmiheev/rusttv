//! Any bytes as an HTTP request head.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    rusttv_remote::fuzzing::request_head(data);
});
