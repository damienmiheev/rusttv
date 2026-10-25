//! Any text as the phones file; whatever parses must write back the same.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(phones) = rusttv_remote::Phones::parse(text) {
        let written = phones.to_toml().unwrap();
        assert_eq!(rusttv_remote::Phones::parse(&written).unwrap(), phones);
    }
});
