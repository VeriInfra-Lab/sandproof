//! Fixed Go 1.26.8 net/http `DetectContentType`; at most 512 bytes, no allocation.
// Copyright 2011 The Go Authors. BSD terms: ../multipart/GO-LICENSE.
pub const SNIFF_BYTES: usize = 512;
#[must_use]
pub fn detect(data: &[u8]) -> &'static str {
    let data = &data[..data.len().min(SNIFF_BYTES)];
    let first = data
        .iter()
        .position(|b| !matches!(b, b'\t' | b'\n' | 12 | b'\r' | b' '))
        .unwrap_or(data.len());
    SIGNATURES
        .iter()
        .find_map(|sig| sig.matches(data, first))
        .unwrap_or("application/octet-stream")
}
pub(super) enum Sig {
    Html(&'static [u8]),
    Exact(&'static [u8], &'static str),
    Masked {
        mask: &'static [u8],
        pattern: &'static [u8],
        skip: bool,
        kind: &'static str,
    },
    Mp4,
    Text,
}
impl Sig {
    fn matches(&self, data: &[u8], first: usize) -> Option<&'static str> {
        match self {
            Self::Html(pattern) => {
                let value = &data[first..];
                (value.len() > pattern.len()
                    && pattern.iter().zip(value).all(|(p, b)| {
                        *p == if p.is_ascii_uppercase() {
                            *b & 0xdf
                        } else {
                            *b
                        }
                    })
                    && matches!(value[pattern.len()], b' ' | b'>'))
                .then_some("text/html; charset=utf-8")
            }
            Self::Exact(pattern, kind) => data.starts_with(pattern).then_some(kind),
            Self::Masked {
                mask,
                pattern,
                skip,
                kind,
            } => {
                let value = if *skip { &data[first..] } else { data };
                (value.len() >= pattern.len()
                    && mask.len() == pattern.len()
                    && value
                        .iter()
                        .zip(mask.iter().zip(*pattern))
                        .all(|(b, (mask, p))| b & mask == *p))
                .then_some(kind)
            }
            Self::Mp4 => mp4(data).then_some("video/mp4"),
            Self::Text => (!data[first..]
                .iter()
                .any(|b| matches!(b, 0..=8 | 11 | 14..=26 | 28..=31)))
            .then_some("text/plain; charset=utf-8"),
        }
    }
}
fn mp4(data: &[u8]) -> bool {
    if data.len() < 12 {
        return false;
    }
    let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let Ok(size) = usize::try_from(size) else {
        return false;
    };
    if size > data.len() || size % 4 != 0 || &data[4..8] != b"ftyp" {
        return false;
    }
    (8..size)
        .step_by(4)
        .any(|offset| offset != 12 && &data[offset..offset + 3] == b"mp4")
}
pub(super) const SIGNATURES: &[Sig] = &[
    Sig::Html(b"<!DOCTYPE HTML"),
    Sig::Html(b"<HTML"),
    Sig::Html(b"<HEAD"),
    Sig::Html(b"<SCRIPT"),
    Sig::Html(b"<IFRAME"),
    Sig::Html(b"<H1"),
    Sig::Html(b"<DIV"),
    Sig::Html(b"<FONT"),
    Sig::Html(b"<TABLE"),
    Sig::Html(b"<A"),
    Sig::Html(b"<STYLE"),
    Sig::Html(b"<TITLE"),
    Sig::Html(b"<B"),
    Sig::Html(b"<BODY"),
    Sig::Html(b"<BR"),
    Sig::Html(b"<P"),
    Sig::Html(b"<!--"),
    Sig::Masked { mask: b"\xFF\xFF\xFF\xFF\xFF", pattern: b"<?xml", skip: true, kind: "text/xml; charset=utf-8" },
    Sig::Exact(b"%PDF-", "application/pdf"),
    Sig::Exact(b"%!PS-Adobe-", "application/postscript"),
    Sig::Masked { mask: b"\xFF\xFF\x00\x00", pattern: b"\xFE\xFF\x00\x00", skip: false, kind: "text/plain; charset=utf-16be" },
    Sig::Masked { mask: b"\xFF\xFF\x00\x00", pattern: b"\xFF\xFE\x00\x00", skip: false, kind: "text/plain; charset=utf-16le" },
    Sig::Masked { mask: b"\xFF\xFF\xFF\x00", pattern: b"\xEF\xBB\xBF\x00", skip: false, kind: "text/plain; charset=utf-8" },
    Sig::Exact(b"\x00\x00\x01\x00", "image/x-icon"),
    Sig::Exact(b"\x00\x00\x02\x00", "image/x-icon"),
    Sig::Exact(b"BM", "image/bmp"),
    Sig::Exact(b"GIF87a", "image/gif"),
    Sig::Exact(b"GIF89a", "image/gif"),
    Sig::Masked { mask: b"\xFF\xFF\xFF\xFF\x00\x00\x00\x00\xFF\xFF\xFF\xFF\xFF\xFF", pattern: b"RIFF\x00\x00\x00\x00WEBPVP", skip: false, kind: "image/webp" },
    Sig::Exact(b"\x89PNG\x0D\x0A\x1A\x0A", "image/png"),
    Sig::Exact(b"\xFF\xD8\xFF", "image/jpeg"),
    Sig::Masked { mask: b"\xFF\xFF\xFF\xFF\x00\x00\x00\x00\xFF\xFF\xFF\xFF", pattern: b"FORM\x00\x00\x00\x00AIFF", skip: false, kind: "audio/aiff" },
    Sig::Masked { mask: b"\xFF\xFF\xFF", pattern: b"ID3", skip: false, kind: "audio/mpeg" },
    Sig::Masked { mask: b"\xFF\xFF\xFF\xFF\xFF", pattern: b"OggS\x00", skip: false, kind: "application/ogg" },
    Sig::Masked { mask: b"\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF", pattern: b"MThd\x00\x00\x00\x06", skip: false, kind: "audio/midi" },
    Sig::Masked { mask: b"\xFF\xFF\xFF\xFF\x00\x00\x00\x00\xFF\xFF\xFF\xFF", pattern: b"RIFF\x00\x00\x00\x00AVI ", skip: false, kind: "video/avi" },
    Sig::Masked { mask: b"\xFF\xFF\xFF\xFF\x00\x00\x00\x00\xFF\xFF\xFF\xFF", pattern: b"RIFF\x00\x00\x00\x00WAVE", skip: false, kind: "audio/wave" },
    Sig::Mp4,
    Sig::Exact(b"\x1A\x45\xDF\xA3", "video/webm"),
    Sig::Masked { mask: b"\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\xFF\xFF", pattern: b"\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00LP", skip: false, kind: "application/vnd.ms-fontobject" },
    Sig::Exact(b"\x00\x01\x00\x00", "font/ttf"),
    Sig::Exact(b"OTTO", "font/otf"),
    Sig::Exact(b"ttcf", "font/collection"),
    Sig::Exact(b"wOFF", "font/woff"),
    Sig::Exact(b"wOF2", "font/woff2"),
    Sig::Exact(b"\x1F\x8B\x08", "application/x-gzip"),
    Sig::Exact(b"PK\x03\x04", "application/zip"),
    Sig::Exact(b"Rar!\x1A\x07\x00", "application/x-rar-compressed"),
    Sig::Exact(b"Rar!\x1A\x07\x01\x00", "application/x-rar-compressed"),
    Sig::Exact(b"\x00\x61\x73\x6D", "application/wasm"),
    Sig::Text,
];
