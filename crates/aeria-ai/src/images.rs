//! Images attached to Angelica's messages.
//!
//! The renderer scales and encodes pasted images before sending them; this
//! module is still the authority on what is accepted. An image is kept as a
//! PNG or JPEG file beside its conversation and referenced from messages by
//! an [`ImageRef`]. Requests carry it inline as a `data:` URL, and only for
//! models that accept images.

use std::collections::HashMap;
use std::fmt::Write as _;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::chat::ChatMessage;

/// Largest accepted image file, so its base64 form stays within the 5 MB
/// that common vision APIs accept per image.
pub const MAX_IMAGE_BYTES: usize = 3_750_000;
/// Longest accepted side in pixels; vision models scale larger images down.
pub const MAX_IMAGE_SIDE: u32 = 2048;
/// Most images in one message.
pub const MAX_IMAGES_PER_MESSAGE: usize = 6;
/// Most images a translation job passes to each worker.
pub const MAX_JOB_IMAGES: usize = 4;

/// An accepted image encoding.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    /// The media type sent to providers.
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }

    /// The file extension used in storage.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }
}

/// One stored image referenced by a message or a job.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageRef {
    /// A UUID, unique within the conversation.
    pub id: String,
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
}

impl ImageRef {
    /// A conservative token estimate for context budgeting: one token per
    /// 28 × 28 pixels, the densest encoding common vision models use, and
    /// at least 85.
    #[must_use]
    pub fn estimated_tokens(&self) -> u64 {
        (u64::from(self.width) * u64::from(self.height))
            .div_ceil(28 * 28)
            .max(85)
    }

    /// Whether `id` is a well-formed image ID, so it can name a file.
    #[must_use]
    pub fn is_valid_id(id: &str) -> bool {
        id.len() == 36 && uuid::Uuid::parse_str(id).is_ok()
    }
}

/// An image that is not accepted.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct ImageError(pub String);

/// Checks an image file and returns its reference under a new ID.
///
/// # Errors
///
/// Returns an error for an empty or oversized file, a format other than PNG
/// or JPEG, an unreadable header, or dimensions outside the limits.
pub fn inspect(bytes: &[u8]) -> Result<ImageRef, ImageError> {
    if bytes.is_empty() {
        return Err(ImageError("the image is empty".to_owned()));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(ImageError(format!(
            "the image is larger than {MAX_IMAGE_BYTES} bytes"
        )));
    }
    let (format, width, height) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let (width, height) = png_size(bytes)?;
        (ImageFormat::Png, width, height)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        let (width, height) = jpeg_size(bytes)?;
        (ImageFormat::Jpeg, width, height)
    } else {
        return Err(ImageError(
            "only PNG and JPEG images are accepted".to_owned(),
        ));
    };
    if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(ImageError(format!(
            "the image is {width}×{height}; each side must be 1 to {MAX_IMAGE_SIDE} pixels"
        )));
    }
    Ok(ImageRef {
        id: uuid::Uuid::new_v4().to_string(),
        format,
        width,
        height,
    })
}

fn png_size(bytes: &[u8]) -> Result<(u32, u32), ImageError> {
    // The signature is followed by the IHDR chunk: length, type, width, height.
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return Err(ImageError("the PNG header is damaged".to_owned()));
    }
    let read =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    Ok((read(16), read(20)))
}

fn jpeg_size(bytes: &[u8]) -> Result<(u32, u32), ImageError> {
    let damaged = || ImageError("the JPEG header is damaged".to_owned());
    let mut at = 2;
    loop {
        // Markers may be padded with fill bytes.
        while bytes.get(at) == Some(&0xFF) && bytes.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if bytes.get(at) != Some(&0xFF) {
            return Err(damaged());
        }
        let marker = *bytes.get(at + 1).ok_or_else(damaged)?;
        at += 2;
        if matches!(marker, 0x01 | 0xD0..=0xD7) {
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            // The image data starts before any frame header was found.
            return Err(damaged());
        }
        let length = usize::from(u16::from_be_bytes([
            *bytes.get(at).ok_or_else(damaged)?,
            *bytes.get(at + 1).ok_or_else(damaged)?,
        ]));
        if length < 2 {
            return Err(damaged());
        }
        // Start-of-frame markers, excluding DHT (C4), JPG (C8), and DAC (CC).
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let frame = bytes.get(at + 2..at + 7).ok_or_else(damaged)?;
            let height = u32::from(u16::from_be_bytes([frame[1], frame[2]]));
            let width = u32::from(u16::from_be_bytes([frame[3], frame[4]]));
            return Ok((width, height));
        }
        at += length;
    }
}

/// Decodes an image the renderer sent as standard base64.
///
/// # Errors
///
/// Returns an error for text that is not base64 or decodes to more than
/// [`MAX_IMAGE_BYTES`].
pub fn decode_base64(text: &str) -> Result<Vec<u8>, ImageError> {
    if text.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err(ImageError(format!(
            "the image is larger than {MAX_IMAGE_BYTES} bytes"
        )));
    }
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| ImageError("the image data is not valid base64".to_owned()))
}

/// The `data:` URL of an image file.
#[must_use]
pub fn data_url(format: ImageFormat, bytes: &[u8]) -> String {
    let mut url = format!("data:{};base64,", format.media_type());
    base64::engine::general_purpose::STANDARD.encode_string(bytes, &mut url);
    url
}

/// Image data for one request, as `data:` URLs by image ID.
#[derive(Clone, Debug, Default)]
pub struct ImagePayloads {
    urls: HashMap<String, String>,
}

impl ImagePayloads {
    /// Adds an image's file content.
    pub fn insert(&mut self, image: &ImageRef, bytes: &[u8]) {
        self.urls
            .insert(image.id.clone(), data_url(image.format, bytes));
    }

    /// The `data:` URL of an image, when it was loaded.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&str> {
        self.urls.get(id).map(String::as_str)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.urls.is_empty()
    }
}

/// The text and the image `data:` URLs of a user message with images.
///
/// The text names the images by ID, so Angelica can pass them to a job, and
/// says which ones could not be sent: to a model without image input none
/// are, and an image whose file is gone is reported as unavailable.
#[must_use]
pub fn user_parts<'a>(
    content: &str,
    images: &[ImageRef],
    payloads: Option<&'a ImagePayloads>,
) -> (String, Vec<&'a str>) {
    let mut text = content.to_owned();
    if images.is_empty() {
        return (text, Vec::new());
    }
    let Some(payloads) = payloads else {
        let _ = write!(
            text,
            "\n\n[{} attached image(s) not shown: the current model does not accept images]",
            images.len()
        );
        return (text, Vec::new());
    };
    let mut parts = Vec::with_capacity(images.len());
    let mut shown = Vec::new();
    let mut missing = 0;
    for image in images {
        if let Some(url) = payloads.get(&image.id) {
            shown.push(image.id.as_str());
            parts.push(url);
        } else {
            missing += 1;
        }
    }
    if !shown.is_empty() {
        let _ = write!(
            text,
            "\n\n[Attached images, in order: {}]",
            shown.join(", ")
        );
    }
    if missing > 0 {
        let _ = write!(
            text,
            "\n\n[{missing} attached image(s) are no longer available]"
        );
    }
    (text, parts)
}

/// Every image referenced by user messages, in order, without duplicates.
#[must_use]
pub fn message_images(messages: &[ChatMessage]) -> Vec<&ImageRef> {
    let mut seen = std::collections::HashSet::new();
    messages
        .iter()
        .filter_map(|message| match message {
            ChatMessage::User { images, .. } => Some(images),
            _ => None,
        })
        .flatten()
        .filter(|image| seen.insert(image.id.as_str()))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal PNG header of the given size; the pixel data is not read.
    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8];
        // An APP0 segment, then a progressive frame header.
        bytes.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46]);
        bytes.extend_from_slice(&[0xFF, 0xFF, 0xC2, 0x00, 0x0B, 0x08]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);
        bytes.extend_from_slice(&[0xFF, 0xDA]);
        bytes
    }

    #[test]
    fn png_and_jpeg_sizes_are_read_from_their_headers() {
        let image = inspect(&png(1280, 720)).expect("png");
        assert_eq!(
            (image.format, image.width, image.height),
            (ImageFormat::Png, 1280, 720)
        );
        assert!(ImageRef::is_valid_id(&image.id));
        let image = inspect(&jpeg(640, 2048)).expect("jpeg");
        assert_eq!(
            (image.format, image.width, image.height),
            (ImageFormat::Jpeg, 640, 2048)
        );
    }

    #[test]
    fn other_formats_damaged_headers_and_oversized_images_are_refused() {
        assert!(inspect(b"").is_err());
        assert!(inspect(b"GIF89a\x01\x00\x01\x00").is_err());
        assert!(inspect(&png(2049, 10)).is_err());
        assert!(inspect(&png(0, 10)).is_err());
        assert!(inspect(&png(10, 10)[..20]).is_err());
        assert!(inspect(&[0xFF, 0xD8, 0xFF, 0xDA]).is_err());
        assert!(inspect(&[0xFF, 0xD8, 0xFF]).is_err());
        let mut large = png(10, 10);
        large.resize(MAX_IMAGE_BYTES + 1, 0);
        assert!(inspect(&large).is_err());
    }

    #[test]
    fn base64_uploads_are_decoded_within_the_size_limit() {
        let bytes = png(3, 3);
        let text = base64::engine::general_purpose::STANDARD.encode(&bytes);
        assert_eq!(decode_base64(&text).expect("decode"), bytes);
        assert!(decode_base64("not base64!").is_err());
        assert!(decode_base64(&"A".repeat(MAX_IMAGE_BYTES.div_ceil(3) * 4 + 4)).is_err());
    }

    #[test]
    fn token_estimates_grow_with_pixels_and_have_a_floor() {
        let image = |width, height| ImageRef {
            id: String::new(),
            format: ImageFormat::Png,
            width,
            height,
        };
        assert_eq!(image(10, 10).estimated_tokens(), 85);
        assert_eq!(image(2048, 1152).estimated_tokens(), 3010);
    }

    #[test]
    fn user_parts_name_sent_images_and_explain_missing_ones() {
        let first = inspect(&png(4, 4)).expect("first");
        let second = inspect(&png(4, 4)).expect("second");
        let images = [first.clone(), second.clone()];
        let (text, parts) = user_parts("Что здесь?", &images, None);
        assert!(parts.is_empty());
        assert!(text.contains("2 attached image(s) not shown"));

        let mut payloads = ImagePayloads::default();
        payloads.insert(&first, &png(4, 4));
        let (text, parts) = user_parts("Что здесь?", &images, Some(&payloads));
        assert_eq!(parts.len(), 1);
        assert!(parts[0].starts_with("data:image/png;base64,iVBOR"));
        assert!(text.contains(&first.id));
        assert!(!text.contains(&second.id));
        assert!(text.contains("1 attached image(s) are no longer available"));
    }
}
