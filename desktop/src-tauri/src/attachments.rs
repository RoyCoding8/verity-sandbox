use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::Path;

/// Maximum allowed image attachment size in bytes (10 MiB).
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// Allowed image MIME types parsed at the file-extension boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageMimeType {
    Png,
    Jpeg,
    Webp,
    Gif,
}

impl ImageMimeType {
    /// Parses a file extension (case-insensitive, without leading dot) into a supported MIME type.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "webp" => Some(Self::Webp),
            "gif" => Some(Self::Gif),
            _ => None,
        }
    }

    /// Returns the canonical MIME type string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
        }
    }
}

/// Validated image attachment ready for Pi JSONL RPC transmission or UI preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAttachment {
    pub filename: String,
    pub mime_type: String,
    pub base64_data: String,
    pub size_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentError {
    MissingFilename,
    UnsupportedExtension(Option<String>),
    TooLarge { size_bytes: u64, max_bytes: usize },
    Io(String),
}

impl fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFilename => write!(f, "image path does not contain a valid filename"),
            Self::UnsupportedExtension(Some(ext)) => write!(
                f,
                "unsupported image extension '.{}' (allowed: .png, .jpg, .jpeg, .webp, .gif)",
                ext
            ),
            Self::UnsupportedExtension(None) => write!(
                f,
                "missing image extension (allowed: .png, .jpg, .jpeg, .webp, .gif)"
            ),
            Self::TooLarge {
                size_bytes,
                max_bytes,
            } => write!(
                f,
                "image size {} bytes exceeds maximum allowed {} bytes",
                size_bytes, max_bytes
            ),
            Self::Io(msg) => write!(f, "failed to read image attachment: {}", msg),
        }
    }
}

impl std::error::Error for AttachmentError {}

const BASE64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Pure RFC 4648 standard Base64 encoder with `=` padding.
pub fn encode_base64(input: &[u8]) -> String {
    if input.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let (chunks, rem) = input.as_chunks::<3>();

    for chunk in chunks {
        let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(BASE64_TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(BASE64_TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(BASE64_TABLE[((n >> 6) & 0x3f) as usize] as char);
        out.push(BASE64_TABLE[(n & 0x3f) as usize] as char);
    }

    match rem.len() {
        1 => {
            let n = (rem[0] as u32) << 16;
            out.push(BASE64_TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(BASE64_TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
            out.push(BASE64_TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(BASE64_TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push(BASE64_TABLE[((n >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => {}
    }

    out
}

/// Pure validation and encoding helper for already-loaded image bytes.
pub fn validate_image_bytes(
    filename: impl Into<String>,
    mime: ImageMimeType,
    bytes: &[u8],
) -> Result<PendingAttachment, AttachmentError> {
    let filename = filename.into();
    if filename.trim().is_empty() {
        return Err(AttachmentError::MissingFilename);
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AttachmentError::TooLarge {
            size_bytes: bytes.len() as u64,
            max_bytes: MAX_IMAGE_BYTES,
        });
    }
    Ok(PendingAttachment {
        filename,
        mime_type: mime.as_str().to_string(),
        base64_data: encode_base64(bytes),
        size_bytes: bytes.len(),
    })
}

/// Boundary loader that validates extension and size before reading an image from disk
/// and returns a Base64-encoded `PendingAttachment`.
pub fn load_and_validate_image(
    path: impl AsRef<Path>,
) -> Result<PendingAttachment, AttachmentError> {
    let path = path.as_ref();

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or(AttachmentError::MissingFilename)?
        .to_string();

    let ext_raw = path.extension().and_then(|e| e.to_str());
    let mime = match ext_raw {
        Some(ext) => ImageMimeType::from_extension(ext)
            .ok_or_else(|| AttachmentError::UnsupportedExtension(Some(ext.to_string())))?,
        None => return Err(AttachmentError::UnsupportedExtension(None)),
    };

    let meta = fs::metadata(path).map_err(|e| AttachmentError::Io(e.to_string()))?;
    if meta.len() > MAX_IMAGE_BYTES as u64 {
        return Err(AttachmentError::TooLarge {
            size_bytes: meta.len(),
            max_bytes: MAX_IMAGE_BYTES,
        });
    }

    let bytes = fs::read(path).map_err(|e| AttachmentError::Io(e.to_string()))?;
    validate_image_bytes(filename, mime, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("verity_attach_test_{}_{}", std::process::id(), id));
        fs::create_dir_all(&dir).expect("create temp test dir");
        let path = dir.join(name);
        fs::write(&path, bytes).expect("write temp image");
        path
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foob"), "Zm9vYg==");
        assert_eq!(encode_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(encode_base64(&[0x00, 0xff, 0x10]), "AP8Q");
    }

    #[test]
    fn loads_all_supported_image_extensions_case_insensitively() {
        let cases = [
            ("shot.png", "image/png"),
            ("photo.jpg", "image/jpeg"),
            ("photo.JPEG", "image/jpeg"),
            ("sticker.webp", "image/webp"),
            ("anim.GIF", "image/gif"),
        ];

        for (name, expected_mime) in cases {
            let path = temp_file(name, b"fake-image-bytes");
            let att = load_and_validate_image(&path).expect("valid image should load");
            assert_eq!(att.filename, name);
            assert_eq!(att.mime_type, expected_mime);
            assert_eq!(att.size_bytes, 16);
            assert_eq!(att.base64_data, encode_base64(b"fake-image-bytes"));
            let _ = fs::remove_dir_all(path.parent().unwrap());
        }
    }

    #[test]
    fn rejects_unsupported_or_missing_extensions() {
        let svg_path = temp_file("vector.svg", b"<svg/>");
        assert_eq!(
            load_and_validate_image(&svg_path),
            Err(AttachmentError::UnsupportedExtension(Some(
                "svg".to_string()
            )))
        );
        let _ = fs::remove_dir_all(svg_path.parent().unwrap());

        let no_ext = temp_file("README", b"hello");
        assert_eq!(
            load_and_validate_image(&no_ext),
            Err(AttachmentError::UnsupportedExtension(None))
        );
        let _ = fs::remove_dir_all(no_ext.parent().unwrap());
    }

    #[test]
    fn enforces_max_image_size_boundary() {
        let exact = vec![0x42u8; MAX_IMAGE_BYTES];
        let ok = validate_image_bytes("exact.png", ImageMimeType::Png, &exact)
            .expect("exact 10 MiB should pass");
        assert_eq!(ok.size_bytes, MAX_IMAGE_BYTES);

        let too_big_path = temp_file("oversize.png", &vec![0u8; MAX_IMAGE_BYTES + 1]);
        assert_eq!(
            load_and_validate_image(&too_big_path),
            Err(AttachmentError::TooLarge {
                size_bytes: (MAX_IMAGE_BYTES + 1) as u64,
                max_bytes: MAX_IMAGE_BYTES,
            })
        );
        let _ = fs::remove_dir_all(too_big_path.parent().unwrap());
    }

    #[test]
    fn reports_io_error_for_missing_file() {
        let missing = std::env::temp_dir().join("verity_nonexistent_image_999999.png");
        let err = load_and_validate_image(&missing).expect_err("missing file must error");
        assert!(matches!(err, AttachmentError::Io(_)));
    }
}
