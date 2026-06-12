//! Transparent decompression by file suffix (visidata parity: .gz/.bz2/.xz/.zst).
//!
//! Compressed files are not seekable, so they take the streaming path
//! (ColumnarStore) instead of the mmap path.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

impl Compression {
    /// Detect compression from the final path suffix.
    pub fn from_path(path: &Path) -> Compression {
        match path.extension().and_then(|e| e.to_str()) {
            Some("gz") => Compression::Gzip,
            Some("bz2") => Compression::Bzip2,
            Some("xz") | Some("lzma") => Compression::Xz,
            Some("zst") => Compression::Zstd,
            _ => Compression::None,
        }
    }

    pub fn is_compressed(&self) -> bool {
        !matches!(self, Compression::None)
    }
}

/// The path with any compression suffix stripped, for filetype dispatch
/// ("data.csv.gz" -> "data.csv").
pub fn inner_path(path: &Path) -> std::path::PathBuf {
    if Compression::from_path(path).is_compressed() {
        path.with_extension("")
    } else {
        path.to_path_buf()
    }
}

/// Open a file with transparent decompression, returning a buffered reader.
pub fn open_reader(path: &Path) -> std::io::Result<Box<dyn Read + Send>> {
    let file = File::open(path)?;
    Ok(match Compression::from_path(path) {
        Compression::None => Box::new(BufReader::with_capacity(1 << 20, file)),
        Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(BufReader::new(file))),
        Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(BufReader::new(file))),
        Compression::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(BufReader::new(
            file,
        ))),
        Compression::Zstd => Box::new(zstd::stream::read::Decoder::new(file)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn suffix_detection() {
        assert_eq!(
            Compression::from_path(Path::new("a.csv.gz")),
            Compression::Gzip
        );
        assert_eq!(
            Compression::from_path(Path::new("a.tsv.zst")),
            Compression::Zstd
        );
        assert_eq!(Compression::from_path(Path::new("a.csv")), Compression::None);
        assert_eq!(
            inner_path(Path::new("data.csv.gz")),
            std::path::PathBuf::from("data.csv")
        );
    }

    #[test]
    fn gzip_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.csv.gz");
        let f = File::create(&p).unwrap();
        let mut enc = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        enc.write_all(b"a,b\n1,2\n").unwrap();
        enc.finish().unwrap();

        let mut r = open_reader(&p).unwrap();
        let mut out = String::new();
        r.read_to_string(&mut out).unwrap();
        assert_eq!(out, "a,b\n1,2\n");
    }

    #[test]
    fn zstd_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.csv.zst");
        let data = zstd::stream::encode_all(&b"x,y\n3,4\n"[..], 1).unwrap();
        std::fs::write(&p, data).unwrap();

        let mut r = open_reader(&p).unwrap();
        let mut out = String::new();
        r.read_to_string(&mut out).unwrap();
        assert_eq!(out, "x,y\n3,4\n");
    }
}
