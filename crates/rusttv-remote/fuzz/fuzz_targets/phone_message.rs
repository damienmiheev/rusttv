//! Any bytes as one decrypted phone message.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = rusttv_remote::protocol::parse_phone(data);
});
