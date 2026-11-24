//! A strict zip reader for packages (PANEL_STUDIO.md § 5.1), in memory.
//!
//! Only the layout `rusttv panel pack` writes is accepted: entries one after
//! another from byte 0, then the central directory, then the end record as
//! the last 22 bytes (no comment). Stored or deflate only; no encryption,
//! data descriptors, zip64, multi-disk or comments. Each local header must
//! agree with its central entry, there are no gaps or overlaps, every CRC is
//! checked, and sizes are enforced on the bytes actually inflated.
//!
//! Inflate is `miniz_oxide` and the CRC `crc32fast`; both are already in the
//! launcher's build (Slint → `image` → `png` → `flate2`).

use crate::error::{Code, PackageError, PackageFile, Position};
use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

/// Largest package zip, in bytes.
pub const MAX_ZIP: usize = 4 << 20;
/// Most bytes all entries may inflate to together.
pub const MAX_INFLATED: usize = 16 << 20;
/// Most entries.
pub const MAX_ENTRIES: usize = 64;
/// Inflated bytes per zip byte, at most.
pub const MAX_RATIO: usize = 20;

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
const END_LEN: usize = 22;
const LOCAL_LEN: usize = 30;
const CENTRAL_LEN: usize = 46;
/// Flag bits a packer may set: deflate options (1, 2) and UTF-8 names (11).
const ALLOWED_FLAGS: u16 = 0b0000_1000_0000_0110;
const ZIP64_EXTRA: u16 = 0x0001;

/// One entry, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

/// Little-endian reads that fail instead of panicking.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b = self.bytes.get(at..at.checked_add(2)?)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b = self.bytes.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn slice(&self, at: usize, len: usize) -> Option<&[u8]> {
        self.bytes.get(at..at.checked_add(len)?)
    }
}

/// The fields a local header and its central entry must agree on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u32,
    size: u32,
    name_len: u16,
}

fn usize_of(n: u32) -> usize {
    // u32 always fits usize on the 32- and 64-bit targets RustTV builds for.
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// Refuses zip64 extra fields and extra fields that don't add up.
fn check_extra(extra: &[u8]) -> Result<(), Code> {
    let mut rest = extra;
    while !rest.is_empty() {
        let [a, b, c, d, tail @ ..] = rest else {
            return Err(Code::BadZip);
        };
        if u16::from_le_bytes([*a, *b]) == ZIP64_EXTRA {
            return Err(Code::ZipUnsupported);
        }
        let len = usize::from(u16::from_le_bytes([*c, *d]));
        rest = tail.get(len..).ok_or(Code::BadZip)?;
    }
    Ok(())
}

fn check_header(header: Header) -> Result<(), Code> {
    if header.flags & 1 != 0 || header.flags & 0x40 != 0 || header.flags & 0x08 != 0 {
        // Encrypted, strongly encrypted, or sizes in a trailing descriptor.
        return Err(Code::ZipUnsupported);
    }
    if header.flags & !ALLOWED_FLAGS != 0 {
        return Err(Code::ZipUnsupported);
    }
    if header.compressed == u32::MAX || header.size == u32::MAX {
        return Err(Code::ZipUnsupported);
    }
    match header.method {
        0 if header.compressed == header.size => Ok(()),
        0 => Err(Code::BadZip),
        8 => Ok(()),
        _ => Err(Code::ZipUnsupported),
    }
}

/// Inflates `data` to exactly `size` bytes and nothing more, using all of
/// `data`.
fn inflate(data: &[u8], size: usize) -> Result<Vec<u8>, Code> {
    let mut out = vec![0; size];
    let mut state = DecompressorOxide::new();
    let flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    let (status, read, written) = decompress(&mut state, data, &mut out, 0, flags);
    if status == TINFLStatus::Done && read == data.len() && written == size {
        Ok(out)
    } else {
        Err(Code::BadZip)
    }
}

/// Reads every entry of `zip`, checking it, with `cap(name)` the most bytes
/// an entry with that name may inflate to.
pub(crate) fn read(
    zip: &[u8],
    cap: impl Fn(&str) -> Result<usize, Code>,
) -> Result<Vec<Entry>, PackageError> {
    let at_file = |code| PackageError::new(code, Position::File).in_file(PackageFile::Zip);
    if zip.len() > MAX_ZIP {
        return Err(at_file(Code::TooLarge));
    }
    let r = Reader { bytes: zip };
    let end = zip
        .len()
        .checked_sub(END_LEN)
        .ok_or_else(|| at_file(Code::NotZip))?;
    if r.u32(end) != Some(END) {
        return Err(at_file(Code::NotZip));
    }
    let field = |at| r.u16(end + at).ok_or_else(|| at_file(Code::BadZip));
    let (disk, cd_disk, here, total, comment) =
        (field(4)?, field(6)?, field(8)?, field(10)?, field(20)?);
    if disk != 0 || cd_disk != 0 || here != total || comment != 0 {
        return Err(at_file(Code::ZipUnsupported));
    }
    let count = usize::from(total);
    if count == 0 {
        // An empty package has no `package.json`.
        return Err(at_file(Code::MissingFile));
    }
    if count > MAX_ENTRIES {
        return Err(at_file(Code::TooMany));
    }
    let cd_size = usize_of(r.u32(end + 12).ok_or_else(|| at_file(Code::BadZip))?);
    let cd_start = usize_of(r.u32(end + 16).ok_or_else(|| at_file(Code::BadZip))?);
    if cd_start.checked_add(cd_size) != Some(end) {
        return Err(at_file(Code::BadZip));
    }

    // The central directory, in order.
    let mut central = Vec::with_capacity(count);
    let mut at = cd_start;
    for index in 0..count {
        let fail =
            |code| PackageError::new(code, Position::Entry { index }).in_file(PackageFile::Zip);
        let read16 = |o: usize| r.u16(at + o).ok_or_else(|| fail(Code::BadZip));
        let read32 = |o: usize| r.u32(at + o).ok_or_else(|| fail(Code::BadZip));
        if read32(0)? != CENTRAL {
            return Err(fail(Code::BadZip));
        }
        let header = Header {
            flags: read16(8)?,
            method: read16(10)?,
            crc: read32(16)?,
            compressed: read32(20)?,
            size: read32(24)?,
            name_len: read16(28)?,
        };
        let (extra_len, comment_len, start_disk) = (read16(30)?, read16(32)?, read16(34)?);
        let offset = read32(42)?;
        check_header(header).map_err(fail)?;
        if comment_len != 0 || start_disk != 0 || offset == u32::MAX {
            return Err(fail(Code::ZipUnsupported));
        }
        let name_at = at + CENTRAL_LEN;
        let name = r
            .slice(name_at, usize::from(header.name_len))
            .ok_or_else(|| fail(Code::BadZip))?;
        let extra = r
            .slice(name_at + name.len(), usize::from(extra_len))
            .ok_or_else(|| fail(Code::BadZip))?;
        check_extra(extra).map_err(fail)?;
        let name = std::str::from_utf8(name).map_err(|_| fail(Code::BadEntryName))?;
        // Names and caps first, so nothing is inflated for a package that
        // is refused for a later entry's name.
        if usize_of(header.size) > cap(name).map_err(fail)? {
            return Err(fail(Code::TooLarge));
        }
        central.push((index, header, name, usize_of(offset)));
        at = name_at + name.len() + extra.len();
    }
    if at != end {
        return Err(at_file(Code::BadZip));
    }

    // Every name once, also ignoring case.
    let mut seen = std::collections::BTreeSet::new();
    for (index, _, name, _) in &central {
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(
                PackageError::new(Code::DuplicateEntry, Position::Entry { index: *index })
                    .in_file(PackageFile::Zip),
            );
        }
    }

    // Declared sizes against the totals before anything is inflated.
    // In u64: 64 sizes near 4 GiB would overflow a 32-bit usize.
    let declared: u64 = central.iter().map(|(_, h, _, _)| u64::from(h.size)).sum();
    let zip_len = u64::try_from(zip.len()).unwrap_or(u64::MAX);
    if declared > MAX_INFLATED as u64 || declared > zip_len.saturating_mul(MAX_RATIO as u64) {
        return Err(at_file(Code::TooMuchInflated));
    }

    // Local headers: back to back from byte 0 up to the central directory.
    let mut by_offset: Vec<_> = central.iter().collect();
    by_offset.sort_by_key(|(_, _, _, offset)| *offset);
    let mut expected = 0;
    let mut entries: Vec<Option<Entry>> = vec![None; count];
    for &&(index, header, name, offset) in &by_offset {
        let fail =
            |code| PackageError::new(code, Position::Entry { index }).in_file(PackageFile::Zip);
        if offset != expected {
            return Err(fail(Code::BadZip));
        }
        let read16 = |o: usize| r.u16(offset + o).ok_or_else(|| fail(Code::BadZip));
        let read32 = |o: usize| r.u32(offset + o).ok_or_else(|| fail(Code::BadZip));
        if read32(0)? != LOCAL {
            return Err(fail(Code::BadZip));
        }
        let local = Header {
            flags: read16(6)?,
            method: read16(8)?,
            crc: read32(14)?,
            compressed: read32(18)?,
            size: read32(22)?,
            name_len: read16(26)?,
        };
        let extra_len = usize::from(read16(28)?);
        if local != header {
            return Err(fail(Code::BadZip));
        }
        let name_at = offset + LOCAL_LEN;
        let local_name = r
            .slice(name_at, name.len())
            .ok_or_else(|| fail(Code::BadZip))?;
        if local_name != name.as_bytes() {
            return Err(fail(Code::BadZip));
        }
        let extra = r
            .slice(name_at + name.len(), extra_len)
            .ok_or_else(|| fail(Code::BadZip))?;
        check_extra(extra).map_err(fail)?;
        let data_at = name_at + name.len() + extra_len;
        let data = r
            .slice(data_at, usize_of(header.compressed))
            .ok_or_else(|| fail(Code::BadZip))?;
        expected = data_at + data.len();
        if expected > cd_start {
            return Err(fail(Code::BadZip));
        }

        let size = usize_of(header.size);
        let bytes = match header.method {
            0 => data.to_vec(),
            _ => inflate(data, size).map_err(fail)?,
        };
        if crc32fast::hash(&bytes) != header.crc {
            return Err(fail(Code::BadCrc));
        }
        if let Some(slot) = entries.get_mut(index) {
            *slot = Some(Entry {
                name: name.to_owned(),
                bytes,
            });
        }
    }
    if expected != cd_start {
        return Err(at_file(Code::BadZip));
    }
    Ok(entries.into_iter().flatten().collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Writes a zip the way `rusttv panel pack` will: entries back to back,
    /// then the directory, then the end record.
    pub(crate) fn zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for (name, bytes, deflate) in files {
            let data = if *deflate {
                miniz_oxide::deflate::compress_to_vec(bytes, 6)
            } else {
                bytes.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let crc = crc32fast::hash(bytes);
            let offset = u32::try_from(out.len()).unwrap();
            let head = |sig: u32, central: bool| {
                let mut h = Vec::new();
                h.extend(sig.to_le_bytes());
                if central {
                    h.extend(20_u16.to_le_bytes());
                }
                h.extend(20_u16.to_le_bytes());
                h.extend(0_u16.to_le_bytes());
                h.extend(method.to_le_bytes());
                h.extend([0; 4]);
                h.extend(crc.to_le_bytes());
                h.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
                h.extend(u32::try_from(bytes.len()).unwrap().to_le_bytes());
                h.extend(u16::try_from(name.len()).unwrap().to_le_bytes());
                h.extend(0_u16.to_le_bytes());
                if central {
                    h.extend([0; 6]); // comment length, disk, internal attributes
                    h.extend([0; 4]);
                    h.extend(offset.to_le_bytes());
                }
                h.extend(name.as_bytes());
                h
            };
            out.extend(head(LOCAL, false));
            out.extend(&data);
            directory.extend(head(CENTRAL, true));
        }
        let cd_start = u32::try_from(out.len()).unwrap();
        out.extend(&directory);
        out.extend(END.to_le_bytes());
        out.extend([0; 4]);
        let n = u16::try_from(files.len()).unwrap();
        out.extend(n.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend(u32::try_from(directory.len()).unwrap().to_le_bytes());
        out.extend(cd_start.to_le_bytes());
        out.extend(0_u16.to_le_bytes());
        out
    }

    fn any(_: &str) -> Result<usize, Code> {
        Ok(MAX_INFLATED)
    }

    fn code(zip: &[u8]) -> Option<Code> {
        read(zip, any).err().map(|e| e.code)
    }

    #[test]
    fn stored_and_deflated_entries_read_back() {
        let text = b"hello hello hello hello".repeat(10);
        let z = zip(&[("a.json", b"{}", false), ("b.json", &text, true)]);
        let entries = read(&z, any).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0],
            Entry {
                name: "a.json".into(),
                bytes: b"{}".to_vec()
            }
        );
        assert_eq!(entries[1].bytes, text);
    }

    #[test]
    fn not_a_zip_or_a_trailing_byte_is_refused() {
        assert_eq!(code(b"hello"), Some(Code::NotZip));
        let mut z = zip(&[("a", b"x", false)]);
        z.push(0);
        assert_eq!(code(&z), Some(Code::NotZip));
        let mut prefixed = vec![0];
        prefixed.extend(zip(&[("a", b"x", false)]));
        assert_eq!(code(&prefixed), Some(Code::BadZip));
    }

    #[test]
    fn a_bad_crc_or_changed_byte_is_caught() {
        let mut z = zip(&[("a", b"xyz", false)]);
        z[LOCAL_LEN + 1] = b'Q';
        assert_eq!(code(&z), Some(Code::BadCrc));
        let mut z = zip(&[("a", &b"abc".repeat(50), true)]);
        let at = LOCAL_LEN + 1 + 2;
        z[at] ^= 0xff;
        assert!(matches!(code(&z), Some(Code::BadZip | Code::BadCrc)));
    }

    #[test]
    fn local_and_central_must_agree() {
        let mut z = zip(&[("a", b"xyz", false)]);
        z[8] = 8; // local method: deflate
        assert_eq!(code(&z), Some(Code::BadZip));
        let mut z = zip(&[("ab", b"xyz", false)]);
        z[LOCAL_LEN] = b'z'; // local name
        assert_eq!(code(&z), Some(Code::BadZip));
    }

    #[test]
    fn duplicates_and_case_collisions_are_refused() {
        assert_eq!(
            code(&zip(&[("a", b"1", false), ("a", b"2", false)])),
            Some(Code::DuplicateEntry)
        );
        assert_eq!(
            code(&zip(&[("a.png", b"1", false), ("A.png", b"2", false)])),
            Some(Code::DuplicateEntry)
        );
    }

    #[test]
    fn unsupported_features_are_refused() {
        let mut z = zip(&[("a", b"x", false)]);
        z[6] = 1; // local flags: encrypted (central too, below)
        let cd = z.len() - END_LEN - (CENTRAL_LEN + 1);
        z[cd + 8] = 1;
        assert_eq!(code(&z), Some(Code::ZipUnsupported));
        let mut z = zip(&[("a", b"x", false)]);
        z[8] = 12; // bzip2, local and central
        z[cd + 10] = 12;
        assert_eq!(code(&z), Some(Code::ZipUnsupported));
        let mut z = zip(&[("a", b"x", false)]);
        let comment = z.len() - 2;
        z[comment] = 1;
        assert_eq!(code(&z), Some(Code::ZipUnsupported));
    }

    #[test]
    fn sizes_are_enforced_on_real_bytes() {
        let big = vec![0_u8; 1 << 20];
        assert_eq!(
            code(&zip(&[("a", &big, true)])),
            Some(Code::TooMuchInflated)
        );
        let ok = vec![7_u8; 10_000];
        assert_eq!(
            read(&zip(&[("a", &ok, false)]), |_| Ok(100))
                .unwrap_err()
                .code,
            Code::TooLarge
        );
        // A size that lies about the deflated data.
        let mut z = zip(&[("a", &b"abcabc".repeat(20), true)]);
        let lie = 22; // local "size"
        z[lie] -= 1;
        let cd = z.len() - END_LEN - (CENTRAL_LEN + 1);
        z[cd + 24] -= 1;
        assert_eq!(code(&z), Some(Code::BadZip));
    }

    /// The central entry of a one-entry zip whose name is one byte long.
    fn central_of_one(z: &[u8]) -> usize {
        z.len() - END_LEN - (CENTRAL_LEN + 1)
    }

    #[test]
    fn descriptors_zip64_and_bad_extras_are_refused() {
        let mut z = zip(&[("a", b"x", false)]);
        let cd = central_of_one(&z);
        z[6] = 8;
        z[cd + 8] = 8;
        assert_eq!(code(&z), Some(Code::ZipUnsupported));
        assert_eq!(check_extra(&[1, 0, 0, 0]), Err(Code::ZipUnsupported));
        assert_eq!(check_extra(&[0x55, 0x54, 9, 0, 1]), Err(Code::BadZip));
        assert_eq!(check_extra(&[0x55, 0x54, 1, 0, 1]), Ok(()));
        assert_eq!(check_extra(&[0x55]), Err(Code::BadZip));
    }

    #[test]
    fn empty_entries_and_padded_streams() {
        assert_eq!(read(&zip(&[("a", b"", true)]), any).unwrap()[0].bytes, b"");
        let data = miniz_oxide::deflate::compress_to_vec(b"abc", 6);
        assert_eq!(inflate(&data, 3), Ok(b"abc".to_vec()));
        let mut padded = data.clone();
        padded.extend([0, 0, 0]);
        assert_eq!(inflate(&padded, 3), Err(Code::BadZip));
        assert_eq!(inflate(&data, 2), Err(Code::BadZip));
        assert_eq!(inflate(&data, 4), Err(Code::BadZip));
    }

    #[test]
    fn the_directory_must_cover_the_entries_exactly() {
        // The directory may list entries in another order than the file.
        let mut z = zip(&[("a", b"1", false), ("b", b"2", false)]);
        let cd = z.len() - END_LEN - 2 * (CENTRAL_LEN + 1);
        let (first, second) = (cd, cd + CENTRAL_LEN + 1);
        let a: Vec<u8> = z[first..second].to_vec();
        let b: Vec<u8> = z[second..second + CENTRAL_LEN + 1].to_vec();
        z[first..first + b.len()].copy_from_slice(&b);
        z[second..second + a.len()].copy_from_slice(&a);
        let names: Vec<_> = read(&z, any).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["b", "a"]);
        // Two directory entries for one local header overlap.
        let mut z = zip(&[("a", b"1", false), ("b", b"2", false)]);
        let second = cd + CENTRAL_LEN + 1;
        z[second + 42..second + 46].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(code(&z), Some(Code::BadZip));
        // Entry counts that disagree.
        let mut z = zip(&[("a", b"1", false)]);
        let here = z.len() - END_LEN + 8;
        z[here] = 2;
        assert_eq!(code(&z), Some(Code::ZipUnsupported));
        // A name that isn't UTF-8.
        let mut z = zip(&[("a", b"1", false)]);
        let cd = central_of_one(&z);
        z[LOCAL_LEN] = 0xff;
        z[cd + CENTRAL_LEN] = 0xff;
        assert_eq!(code(&z), Some(Code::BadEntryName));
        // Central entry comment.
        let mut z = zip(&[("a", b"1", false)]);
        z[cd + 32] = 1;
        assert_eq!(code(&z), Some(Code::ZipUnsupported));
    }

    #[test]
    fn huge_declared_sizes_dont_overflow() {
        let mut z = zip(&[("a", b"x", true), ("b", b"y", true), ("c", b"z", true)]);
        let cd = z.len() - END_LEN - 3 * (CENTRAL_LEN + 1);
        for i in 0..3 {
            let size = cd + i * (CENTRAL_LEN + 1) + 24;
            z[size..size + 4].copy_from_slice(&0xFFFF_FFFE_u32.to_le_bytes());
        }
        assert_eq!(code(&z), Some(Code::TooLarge));
        // Summed in u64, so this also holds where usize is 32 bits.
        let no_cap = read(&z, |_| Ok(usize::MAX)).unwrap_err().code;
        assert_eq!(no_cap, Code::TooMuchInflated);
    }

    #[test]
    fn an_empty_zip_has_no_package_json() {
        assert_eq!(code(&zip(&[])), Some(Code::MissingFile));
    }

    #[test]
    fn a_gap_before_the_directory_is_refused() {
        let z = zip(&[("a", b"1", false)]);
        let cd = central_of_one(&z);
        let mut gapped = z[..cd].to_vec();
        gapped.push(0);
        gapped.extend(&z[cd..]);
        let offset = gapped.len() - 6;
        let start = u32::try_from(cd + 1).unwrap();
        gapped[offset..offset + 4].copy_from_slice(&start.to_le_bytes());
        assert_eq!(code(&gapped), Some(Code::BadZip));
    }

    #[test]
    fn zip64_markers_are_refused() {
        let mut z = zip(&[("a", b"x", true)]);
        let cd = central_of_one(&z);
        for at in [18, cd + 20] {
            z[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        assert_eq!(code(&z), Some(Code::ZipUnsupported));
        let mut z = zip(&[("a", b"x", false)]);
        let cd = central_of_one(&z);
        z[cd + 42..cd + 46].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(code(&z), Some(Code::ZipUnsupported));
    }

    #[test]
    fn too_many_entries_are_refused() {
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("f{i}")).collect();
        let files: Vec<(&str, &[u8], bool)> = names
            .iter()
            .map(|n| (n.as_str(), &b"x"[..], false))
            .collect();
        assert_eq!(code(&zip(&files)), Some(Code::TooMany));
    }

    #[test]
    fn errors_never_contain_names() {
        let error = read(
            &zip(&[("secret", b"1", false), ("secret", b"2", false)]),
            any,
        )
        .unwrap_err();
        assert!(!error.to_string().contains("secret"), "{error}");
    }
}
