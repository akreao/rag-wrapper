//! Writes THOR patch archives, the format the players' patcher
//! ([rpatchur](https://github.com/L1nkZ/rpatchur)) downloads and applies.
//! Follows rpatchur's own writer (`gruf::thor::ThorArchiveBuilder`, which
//! its `mkpatch` uses), so the patcher reads what this writes.
//!
//! Layout, little-endian throughout:
//! - header: the magic, `use_grf_merging` (u8), the entry count (u32), the
//!   mode (i16, 48 for "multiple files"), the target GRF's name (u8 length
//!   and bytes), then the file table's compressed size and offset (u32 each)
//! - each file's zlib-compressed content
//! - the zlib-compressed file table: per entry the path (u8 length and
//!   bytes, Windows separators), flags (u8, 1 for a removal), and for a file
//!   its offset, compressed size and size (u32 each)
//!
//! `data.integrity` lists each file's CRC32, so the patcher (which has
//! `check_integrity` on) can reject a damaged download.

use std::io::{self, Read, Seek, SeekFrom, Write};

use flate2::write::ZlibEncoder;
use flate2::{Compression, Crc};

const MAGIC: &[u8; 24] = b"ASSF (C) 2007 Aeomin DEV";
const MODE_MULTIPLE_FILES: i16 = 48;
const INTEGRITY_FILE: &[u8] = b"data.integrity";

pub struct ThorWriter<W: Write + Seek> {
    out: W,
    grf: Option<Vec<u8>>,
    entries: Vec<Entry>,
}

struct Entry {
    path: Vec<u8>,
    /// Offset, compressed size, size and CRC32; `None` for a removal.
    file: Option<(u32, u32, u32, u32)>,
}

impl<W: Write + Seek> ThorWriter<W> {
    /// A patch for the client folder, or with `grf` set, for that GRF.
    pub fn new(mut out: W, grf: Option<&[u8]>) -> io::Result<Self> {
        let grf = grf.map(<[u8]>::to_vec);
        let header_len = header(true, 0, grf.as_deref().unwrap_or_default(), 0, 0)?.len();
        out.write_all(&vec![0; header_len])?;
        Ok(Self { out, grf, entries: Vec::new() })
    }

    /// Adds a file. `path` is relative, with `\` separators, in the
    /// client's encoding.
    pub fn add(&mut self, path: &[u8], content: &[u8]) -> io::Result<()> {
        self.add_from(path, content).map(|_| ())
    }

    /// Adds a file read from `content`, compressed straight into the
    /// archive, so a large file is never held in memory. Returns its size.
    pub fn add_from(&mut self, path: &[u8], mut content: impl Read) -> io::Result<u64> {
        let offset = self.out.stream_position()?;
        let mut crc = Crc::new();
        let mut size = 0u64;
        let mut zlib = ZlibEncoder::new(&mut self.out, Compression::default());
        let mut buf = vec![0; 1 << 16];
        loop {
            let n = match content.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            crc.update(&buf[..n]);
            zlib.write_all(&buf[..n])?;
            size += n as u64;
        }
        zlib.finish()?;
        let end = self.out.stream_position()?;
        self.entries.push(Entry {
            path: path.to_vec(),
            file: Some((u32_of(offset)?, u32_of(end - offset)?, u32_of(size)?, crc.sum())),
        });
        Ok(size)
    }

    /// Deletes a file from the client folder or GRF.
    pub fn remove(&mut self, path: &[u8]) {
        self.entries.push(Entry { path: path.to_vec(), file: None });
    }

    /// Writes the integrity list, the file table and the header.
    pub fn finish(mut self) -> io::Result<W> {
        let mut integrity = Vec::new();
        for e in &self.entries {
            if let Some((.., crc)) = e.file {
                integrity.extend_from_slice(&e.path);
                integrity.extend_from_slice(format!("=0x{crc:08x}\r\n").as_bytes());
            }
        }
        self.add(INTEGRITY_FILE, &integrity)?;

        let mut table = Vec::new();
        for e in &self.entries {
            table.push(u8_len(&e.path)?);
            table.extend_from_slice(&e.path);
            match e.file {
                None => table.push(1),
                Some((offset, compressed, size, _)) => {
                    table.push(0);
                    for n in [offset, compressed, size] {
                        table.extend_from_slice(&n.to_le_bytes());
                    }
                }
            }
        }
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib.write_all(&table)?;
        let table = zlib.finish()?;
        let table_offset = self.out.stream_position()?;
        self.out.write_all(&table)?;

        let grf = self.grf.as_deref();
        let head = header(grf.is_some(), self.entries.len(), grf.unwrap_or_default(), table.len(), table_offset)?;
        self.out.seek(SeekFrom::Start(0))?;
        self.out.write_all(&head)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

fn header(grf_merging: bool, count: usize, grf: &[u8], table_len: usize, table_offset: u64) -> io::Result<Vec<u8>> {
    let mut h = MAGIC.to_vec();
    h.push(grf_merging as u8);
    h.extend_from_slice(&u32_of(count)?.to_le_bytes());
    h.extend_from_slice(&MODE_MULTIPLE_FILES.to_le_bytes());
    h.push(u8_len(grf)?);
    h.extend_from_slice(grf);
    h.extend_from_slice(&u32_of(table_len)?.to_le_bytes());
    h.extend_from_slice(&u32_of(table_offset)?.to_le_bytes());
    Ok(h)
}

fn u32_of<N: TryInto<u32>>(n: N) -> io::Result<u32> {
    n.try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "A patch can't be larger than 4 GB."))
}

fn u8_len(bytes: &[u8]) -> io::Result<u8> {
    u8::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "A file path in a patch can't be longer than 255 bytes."))
}

/// Reads a THOR archive back, the way rpatchur's reader does. For tests.
#[cfg(test)]
pub mod read {
    use std::collections::BTreeMap;
    use std::io::Read;

    use flate2::read::ZlibDecoder;

    pub struct Archive {
        pub grf_merging: bool,
        pub grf: Vec<u8>,
        /// Path to content; `None` for a removal.
        pub files: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    }

    fn u32_at(b: &[u8], at: usize) -> usize {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize
    }

    fn inflate(b: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        ZlibDecoder::new(b).read_to_end(&mut out).unwrap();
        out
    }

    pub fn archive(b: &[u8]) -> Archive {
        assert_eq!(&b[..24], super::MAGIC);
        let grf_merging = b[24] == 1;
        let count = u32_at(b, 25);
        assert_eq!(i16::from_le_bytes([b[29], b[30]]), 48);
        let name_len = b[31] as usize;
        let grf = b[32..32 + name_len].to_vec();
        let at = 32 + name_len;
        let (table_len, table_offset) = (u32_at(b, at), u32_at(b, at + 4));
        let table = inflate(&b[table_offset..table_offset + table_len]);

        let mut files = BTreeMap::new();
        let mut i = 0;
        while i < table.len() {
            let len = table[i] as usize;
            let path = table[i + 1..i + 1 + len].to_vec();
            i += 1 + len;
            let flags = table[i];
            i += 1;
            if flags & 1 == 1 {
                files.insert(path, None);
                continue;
            }
            let (offset, compressed, size) = (u32_at(&table, i), u32_at(&table, i + 4), u32_at(&table, i + 8));
            i += 12;
            let content = inflate(&b[offset..offset + compressed]);
            assert_eq!(content.len(), size);
            files.insert(path, Some(content));
        }
        assert_eq!(files.len(), count);
        Archive { grf_merging, grf, files }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn build(grf: Option<&[u8]>, add: &[(&[u8], &[u8])], remove: &[&[u8]]) -> Vec<u8> {
        let mut w = ThorWriter::new(Cursor::new(Vec::new()), grf).unwrap();
        for (path, content) in add {
            w.add(path, content).unwrap();
        }
        for path in remove {
            w.remove(path);
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn round_trips_files_and_removals() {
        let bytes = build(None, &[(b"rag-wrapper.exe", b"MZ..."), (b"System\\a.lub", b"")], &[b"old.txt"]);
        let a = read::archive(&bytes);
        assert!(!a.grf_merging);
        assert!(a.grf.is_empty());
        assert_eq!(a.files[&b"rag-wrapper.exe"[..]].as_deref(), Some(&b"MZ..."[..]));
        assert_eq!(a.files[&b"System\\a.lub"[..]].as_deref(), Some(&b""[..]));
        assert_eq!(a.files[&b"old.txt"[..]], None);
    }

    #[test]
    fn streams_a_file_larger_than_its_buffer() {
        let content: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut w = ThorWriter::new(Cursor::new(Vec::new()), None).unwrap();
        assert_eq!(w.add_from(b"bgm.mp3", content.as_slice()).unwrap(), 300_000);
        let a = read::archive(&w.finish().unwrap().into_inner());
        assert_eq!(a.files[&b"bgm.mp3"[..]].as_deref(), Some(content.as_slice()));
    }

    #[test]
    fn names_the_target_grf() {
        let a = read::archive(&build(Some(b"rag-wrapper.grf"), &[(b"data\\x.txt", b"x")], &[]));
        assert!(a.grf_merging);
        assert_eq!(a.grf, b"rag-wrapper.grf");
    }

    // rpatchur checks each file against data.integrity before applying.
    #[test]
    fn lists_checksums_in_data_integrity() {
        let a = read::archive(&build(None, &[(b"a.txt", b"hello")], &[b"gone.txt"]));
        let integrity = a.files[INTEGRITY_FILE].clone().unwrap();
        let mut crc = Crc::new();
        crc.update(b"hello");
        assert_eq!(integrity, format!("a.txt=0x{:08x}\r\n", crc.sum()).into_bytes());
        assert_eq!(crc.sum(), 0x3610a686);
    }

    #[test]
    fn rejects_long_paths() {
        let mut w = ThorWriter::new(Cursor::new(Vec::new()), None).unwrap();
        w.add(&[b'a'; 256], b"").unwrap();
        assert!(w.finish().is_err());
    }
}
