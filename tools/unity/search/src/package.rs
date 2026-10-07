//! Reads files out of a Unity Mac package over the network. The package
//! itself never lands on disk. A package is a xar archive whose `Payload`
//! is a gzip'd cpio. The table of contents sits at the front, so the first
//! megabyte says where the payload starts. A second range request streams
//! the payload from there and stops once every wanted file is out.

use std::{
    fs, io,
    io::{Read, Write},
    path::Path,
    thread,
    time::Duration,
};

use flate2::read::{GzDecoder, ZlibDecoder};

const AGENT: &str = "unity-search";

/// Opens a URL over a byte range, from `first` to the end when `last` is
/// `None`.
fn open(url: &str, first: u64, last: Option<u64>) -> io::Result<Box<dyn Read + Send + Sync>> {
    let range = match last {
        Some(last) => format!("bytes={first}-{last}"),
        None => format!("bytes={first}-"),
    };
    let response = ureq::get(url)
        .header("User-Agent", AGENT)
        .header("Range", &range)
        .call()
        .map_err(io::Error::other)?;
    Ok(Box::new(response.into_body().into_reader()))
}

/// Finds where the `Payload` starts in a package, and how long it is, from
/// the xar table of contents.
pub(crate) fn payload_at(url: &str) -> io::Result<(u64, u64)> {
    let mut head = Vec::new();
    open(url, 0, Some((1 << 20) - 1))?.read_to_end(&mut head)?;
    payload_in_header(&head)
}

/// Parses a xar header and its table of contents for the `Payload` entry.
fn payload_in_header(head: &[u8]) -> io::Result<(u64, u64)> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    if head.len() < 28 || &head[..4] != b"xar!" {
        return Err(bad("not a xar archive"));
    }
    let header_size = u16::from_be_bytes([head[4], head[5]]) as usize;
    let toc_size = u64::from_be_bytes(head[8..16].try_into().unwrap()) as usize;
    let toc = head
        .get(header_size..header_size + toc_size)
        .ok_or_else(|| bad("table of contents past the first megabyte"))?;
    let mut xml = String::new();
    ZlibDecoder::new(toc).read_to_string(&mut xml)?;
    let document = roxmltree::Document::parse(&xml).map_err(io::Error::other)?;
    for file in document.descendants().filter(|n| n.has_tag_name("file")) {
        let name = file
            .children()
            .find(|n| n.has_tag_name("name"))
            .and_then(|n| n.text())
            .unwrap_or("");
        let Some(data) = file.children().find(|n| n.has_tag_name("data")) else {
            continue;
        };
        if !name.ends_with("Payload") {
            continue;
        }
        let number = |tag: &str| -> Option<u64> {
            data.children()
                .find(|n| n.has_tag_name(tag))?
                .text()?
                .trim()
                .parse()
                .ok()
        };
        let offset = number("offset").ok_or_else(|| bad("payload without an offset"))?;
        let length = number("length").ok_or_else(|| bad("payload without a length"))?;
        return Ok(((header_size + toc_size) as u64 + offset, length));
    }
    Err(bad("no Payload in the table of contents"))
}

/// Reads a byte range and reopens it from the next byte when the server
/// drops the connection. The range feeds a decompressor that can't restart
/// halfway, so starting over from the first byte is not an option.
struct Resumable {
    url: String,
    first: u64,
    last: u64,
    read: u64,
    inner: Box<dyn Read + Send + Sync>,
}

impl Resumable {
    const TRIES: u32 = 8;

    fn new(url: &str, first: u64, last: u64) -> io::Result<Self> {
        Ok(Self {
            url: url.to_string(),
            first,
            last,
            read: 0,
            inner: open(url, first, Some(last))?,
        })
    }
}

impl Read for Resumable {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        for attempt in 1..=Self::TRIES {
            match self.inner.read(buf) {
                Ok(n) => {
                    self.read += n as u64;
                    return Ok(n);
                }
                Err(_) if attempt < Self::TRIES => {
                    thread::sleep(Duration::from_secs(2 * attempt as u64));
                    self.inner = open(&self.url, self.first + self.read, Some(self.last))?;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }
}

/// Describes one entry of a cpio stream: its path and how many bytes follow.
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) size: u64,
    /// Bytes of padding after the body, in the newc format.
    pad: u64,
}

/// Reads the next cpio header. Returns `None` at the trailer or at the end
/// of the stream. Unity's packages use the odc format (ascii octal) or the
/// newc format (ascii hex, 4-byte aligned), and this reads both.
pub(crate) fn next_entry(stream: &mut impl Read) -> io::Result<Option<Entry>> {
    let mut magic = [0; 6];
    if let Err(error) = stream.read_exact(&mut magic) {
        return match error.kind() {
            io::ErrorKind::UnexpectedEof => Ok(None),
            _ => Err(error),
        };
    }
    let bad = |what: String| io::Error::new(io::ErrorKind::InvalidData, what);
    let (name_size, size, pad) = match &magic {
        b"070707" => {
            let mut header = [0; 70];
            stream.read_exact(&mut header)?;
            let field = |from: usize, to: usize| {
                u64::from_str_radix(std::str::from_utf8(&header[from..to]).unwrap_or("x"), 8)
                    .map_err(|_| bad("bad odc header".to_string()))
            };
            (field(53, 59)?, field(59, 70)?, 0)
        }
        b"070701" | b"070702" => {
            let mut header = [0; 104];
            stream.read_exact(&mut header)?;
            let field = |index: usize| {
                let from = index * 8;
                u64::from_str_radix(
                    std::str::from_utf8(&header[from..from + 8]).unwrap_or("x"),
                    16,
                )
                .map_err(|_| bad("bad newc header".to_string()))
            };
            let (size, name_size) = (field(6)?, field(11)?);
            // The name is padded so the body starts 4-byte aligned, and the
            // body is padded the same way.
            let name_pad = (4 - (110 + name_size) % 4) % 4;
            (name_size + name_pad, size, (4 - size % 4) % 4)
        }
        other => return Err(bad(format!("unknown cpio magic {other:?}"))),
    };
    let mut name = vec![0; name_size as usize];
    stream.read_exact(&mut name)?;
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    let name = String::from_utf8_lossy(&name[..end]).into_owned();
    if name == "TRAILER!!!" {
        return Ok(None);
    }
    Ok(Some(Entry { name, size, pad }))
}

/// Copies one entry's body to `dest`, or skips it when there is nothing to
/// write to, then the padding after it.
pub(crate) fn pull(stream: &mut impl Read, entry: &Entry, dest: Option<&Path>) -> io::Result<()> {
    let mut body = stream.take(entry.size);
    match dest {
        Some(path) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = fs::File::create(path)?;
            io::copy(&mut body, &mut file)?;
            file.flush()?;
        }
        None => {
            io::copy(&mut body, &mut io::sink())?;
        }
    }
    io::copy(&mut stream.take(entry.pad), &mut io::sink())?;
    Ok(())
}

/// Pulls the wanted files out of one package into `into`, each under its
/// own file name. An entry matches a wanted path when the entry's path ends
/// with it. Returns how many bytes of the package the stream read.
pub(crate) fn take(url: &str, wanted: &[String], into: &Path) -> io::Result<u64> {
    let (start, length) = payload_at(url)?;
    let mut left: Vec<&String> = wanted.iter().collect();
    let mut stream = GzDecoder::new(Resumable::new(url, start, start + length - 1)?);
    while let Some(entry) = next_entry(&mut stream)? {
        if left.is_empty() {
            break;
        }
        let hit = left
            .iter()
            .position(|want| entry.name.ends_with(want.as_str()));
        let dest = match hit {
            Some(index) if entry.size > 0 => {
                let want = left.remove(index);
                let file = want.rsplit('/').next().unwrap_or(want);
                Some(into.join(file))
            }
            _ => None,
        };
        pull(&mut stream, &entry, dest.as_deref())?;
    }
    if let Some(missing) = left.first() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{missing} is not in the package"),
        ));
    }
    Ok(stream.get_ref().read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::ZlibEncoder, Compression};

    // Builds a xar header by hand from the spec: the magic, the header
    // size, the version, the compressed and uncompressed table sizes, and
    // the checksum kind, then the zlib'd table of contents.
    fn xar(toc: &str) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(toc.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(b"xar!");
        out.extend_from_slice(&28_u16.to_be_bytes());
        out.extend_from_slice(&1_u16.to_be_bytes());
        out.extend_from_slice(&(compressed.len() as u64).to_be_bytes());
        out.extend_from_slice(&(toc.len() as u64).to_be_bytes());
        out.extend_from_slice(&1_u32.to_be_bytes());
        out.extend_from_slice(&compressed);
        out
    }

    #[test]
    fn the_payload_offset_counts_from_the_end_of_the_table() {
        let toc = "<xar><toc><file><name>Bom</name><data><offset>0</offset><length>5</length></data></file>\
                   <file><name>Payload</name><data><offset>100</offset><length>7</length></data></file></toc></xar>";
        let head = xar(toc);
        let toc_size = head.len() - 28;
        assert_eq!(
            payload_in_header(&head).unwrap(),
            ((28 + toc_size + 100) as u64, 7)
        );
    }

    #[test]
    fn a_package_without_a_payload_is_an_error() {
        let head = xar("<xar><toc><file><name>Bom</name></file></toc></xar>");
        assert!(payload_in_header(&head).is_err());
        assert!(payload_in_header(b"not xar").is_err());
    }

    // One odc entry: the magic, 7 six-digit octal fields, the 11-digit
    // mtime, the name size, the 11-digit file size, then the name with its
    // NUL.
    fn odc(name: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "070707{}{:011o}{:06o}{:011o}{}\0",
            "000000".repeat(7),
            0,
            name.len() + 1,
            body.len(),
            name
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    // One newc entry: the magic, 13 eight-digit hex fields, then the name
    // with its NUL, each part padded to 4 bytes.
    fn newc(name: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "070701{}{:08x}{}{:08x}00000000{}\0",
            "00000000".repeat(6),
            body.len(),
            "00000000".repeat(4),
            name.len() + 1,
            name
        )
        .into_bytes();
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out.extend_from_slice(body);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    #[test]
    fn reads_odc_entries_up_to_the_trailer() {
        let stream = [odc("./a/b.h", b"hello"), odc("TRAILER!!!", b"")].concat();
        let mut cursor = io::Cursor::new(stream);
        let entry = next_entry(&mut cursor).unwrap().unwrap();
        assert_eq!((entry.name.as_str(), entry.size), ("./a/b.h", 5));
        pull(&mut cursor, &entry, None).unwrap();
        assert!(next_entry(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn reads_newc_entries_with_their_padding() {
        let stream = [
            newc("./x", b"12345"),
            newc("./y.h", b"ab"),
            newc("TRAILER!!!", b""),
        ]
        .concat();
        let mut cursor = io::Cursor::new(stream);
        let first = next_entry(&mut cursor).unwrap().unwrap();
        assert_eq!((first.name.as_str(), first.size), ("./x", 5));
        pull(&mut cursor, &first, None).unwrap();
        let second = next_entry(&mut cursor).unwrap().unwrap();
        assert_eq!((second.name.as_str(), second.size), ("./y.h", 2));
        let dir = std::env::temp_dir().join("unity-search-newc");
        let dest = dir.join("y.h");
        pull(&mut cursor, &second, Some(&dest)).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"ab");
        assert!(next_entry(&mut cursor).unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
