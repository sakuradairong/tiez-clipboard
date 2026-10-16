//! Prepare every payload before changing the system clipboard. No UI or Tauri.
use crate::domain::models::ClipboardEntry;
use crate::utils::{
    extract_first_image_data_url_from_html, split_rich_html_and_image_fallback,
    split_rich_html_and_named_formats,
};
use crate::write_helpers::{
    generate_cf_html, is_animated_clipboard_image_bytes, should_attach_png_clipboard_format,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::io::Cursor;
use std::path::Path;
use std::time::Duration;
use tiez_native_platform::clipboard_formats::{self as formats, NamedClipboardFormat};

const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_RGBA_BYTES: u64 = 128 * 1024 * 1024;
struct PreparedImage {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
    animated: Option<Vec<u8>>,
    png: Option<Vec<u8>>,
}
enum Payload {
    Text(String),
    Files(Vec<String>),
    Image(PreparedImage),
    Rich {
        text: String,
        cf_html: String,
        named: Vec<NamedClipboardFormat>,
        image: Option<PreparedImage>,
    },
}
/// Intentionally has no Debug implementation: payloads may contain private data.
pub struct PreparedWrite(Payload);
pub struct WriteReceipt {
    pub sequence: u32,
    /// Some animated targets require the production CF_HDROP compatibility file.
    pub temporary_image_path: Option<String>,
}
#[derive(Debug)]
pub struct WriteError {
    pub code: &'static str,
    /// A failed multi-format write can already have changed the clipboard.
    pub clipboard_changed: bool,
}

fn image_bytes(payload: &str, roots: &[std::path::PathBuf]) -> Result<Vec<u8>, &'static str> {
    if let Some(data) = payload.strip_prefix("data:image/") {
        let (_, encoded) = data.split_once(";base64,").ok_or("image_encoding")?;
        if encoded.len() > MAX_IMAGE_BYTES * 4 / 3 + 4 {
            return Err("image_too_large");
        }
        let bytes = STANDARD.decode(encoded).map_err(|_| "image_encoding")?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err("image_too_large");
        }
        return Ok(bytes);
    }
    let value = match payload.strip_prefix("file://") {
        Some(uri) => urlencoding::decode(uri).map_err(|_| "image_path")?,
        None => std::borrow::Cow::Borrowed(payload),
    };
    let value = if value.starts_with('/') && value.as_bytes().get(2) == Some(&b':') {
        &value[1..]
    } else {
        &value
    };
    if !Path::new(value).is_absolute() {
        return Err("image_path");
    }
    crate::local_image_resource::read_local_image(Path::new(value), roots, MAX_IMAGE_BYTES)
        .map(|image| image.bytes)
        .ok_or("image_resource_unavailable")
}
fn validate_dimensions(width: u32, height: u32) -> Result<(), &'static str> {
    if width == 0
        || height == 0
        || width > 16_384
        || height > 16_384
        || u64::from(width) * u64::from(height) * 4 > MAX_RGBA_BYTES
    {
        return Err("image_dimensions");
    }
    Ok(())
}
fn prepare_image(bytes: Vec<u8>) -> Result<PreparedImage, &'static str> {
    let animated = is_animated_clipboard_image_bytes(&bytes);
    let mut reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| "image_decode")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| "image_decode")?;
    validate_dimensions(image.width(), image.height())?;
    let png = if should_attach_png_clipboard_format(animated) {
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .map_err(|_| "image_encode")?;
        Some(encoded.into_inner())
    } else {
        None
    };
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(PreparedImage {
        width: width as usize,
        height: height as usize,
        rgba: rgba.into_raw(),
        animated: animated.then_some(bytes),
        png,
    })
}
impl PreparedWrite {
    /// `image_roots` must be chosen by the owner for an explicit stored image.
    /// Rich preview fallback images always use the core's scoped managed/temp roots.
    pub fn prepare(
        entry: &ClipboardEntry,
        plain_text: bool,
        image_roots: &[std::path::PathBuf],
    ) -> Result<Self, &'static str> {
        if plain_text {
            return Self::text(&entry.content);
        }
        let payload = match entry.content_type.as_str() {
            "text" | "code" | "url" => return Self::text(&entry.content),
            "file" | "video" => {
                let paths = entry
                    .content
                    .lines()
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if paths.is_empty()
                    || paths
                        .iter()
                        .any(|path| path.contains('\0') || !Path::new(path).is_absolute())
                {
                    return Err("file_paths");
                }
                Payload::Files(paths)
            }
            "image" => Payload::Image(prepare_image(image_bytes(&entry.content, image_roots)?)?),
            "rich_text" => {
                let Some(html) = entry.html_content.as_deref() else {
                    return Self::text(&entry.content);
                };
                let (without_named, named) = split_rich_html_and_named_formats(html);
                let (clean, fallback) = split_rich_html_and_image_fallback(&without_named);
                let html = clean.as_str();
                let fallback = if html.trim().is_empty() && entry.content.trim().is_empty() {
                    fallback.or_else(|| extract_first_image_data_url_from_html(html))
                } else {
                    None
                };
                let image = fallback
                    .map(|payload| {
                        image_bytes(
                            &payload,
                            &crate::image_roots::current_clipboard_image_roots(),
                        )
                        .and_then(prepare_image)
                    })
                    .transpose()?;
                // UTF-16 text and CF_HTML are NUL terminated by the native writer.
                if entry.content.contains('\0') || html.contains('\0') {
                    return Err("text_nul");
                }
                Payload::Rich {
                    text: entry.content.clone(),
                    cf_html: generate_cf_html(html),
                    named,
                    image,
                }
            }
            _ => return Err("content_type"),
        };
        Ok(Self(payload))
    }
    fn text(text: &str) -> Result<Self, &'static str> {
        if text.contains('\0') {
            return Err("text_nul");
        }
        Ok(Self(Payload::Text(text.into())))
    }
    /// Fingerprint the actual prepared raster, including an image-only rich fallback.
    pub fn image_visual_hash(&self) -> Option<i64> {
        let image = match &self.0 {
            Payload::Image(image) => Some(image),
            Payload::Rich { image, .. } => image.as_ref(),
            _ => None,
        }?;
        crate::database::calc_image_hash_from_rgba(
            image.width as u32,
            image.height as u32,
            &image.rgba,
        )
    }

    /// All serialization/decoding/path validation is complete before this call.
    /// OS ownership is multi-format, so later failures are explicitly reported.
    pub fn commit(self) -> Result<WriteReceipt, WriteError> {
        let before = formats::get_clipboard_sequence_number();
        let result = self.write_to(&mut WindowsSink);
        result
            .map(|temporary_image_path| WriteReceipt {
                sequence: formats::get_clipboard_sequence_number(),
                temporary_image_path,
            })
            .map_err(|code| WriteError {
                code,
                clipboard_changed: formats::get_clipboard_sequence_number() != before,
            })
    }
    fn write_to(self, sink: &mut impl Sink) -> Result<Option<String>, &'static str> {
        match self.0 {
            Payload::Text(text) => {
                sink.text(&text)?;
                Ok(None)
            }
            Payload::Files(paths) => {
                sink.files(paths)?;
                Ok(None)
            }
            Payload::Image(image) => sink.image(image),
            Payload::Rich {
                text,
                cf_html,
                named,
                image,
            } => {
                let append = image.is_some();
                let path = image.map(|image| sink.image(image)).transpose()?.flatten();
                sink.rich(&text, &cf_html, &named, append)?;
                Ok(path)
            }
        }
    }
}
trait Sink {
    fn text(&mut self, text: &str) -> Result<(), &'static str>;
    fn files(&mut self, paths: Vec<String>) -> Result<(), &'static str>;
    fn image(&mut self, image: PreparedImage) -> Result<Option<String>, &'static str>;
    fn rich(
        &mut self,
        text: &str,
        html: &str,
        named: &[NamedClipboardFormat],
        append: bool,
    ) -> Result<(), &'static str>;
}
struct WindowsSink;
impl Sink for WindowsSink {
    fn text(&mut self, text: &str) -> Result<(), &'static str> {
        for attempt in 0..3 {
            let result =
                arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text));
            if result.is_ok() {
                return Ok(());
            }
            if attempt < 2 {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        Err("clipboard_text_write")
    }
    fn files(&mut self, paths: Vec<String>) -> Result<(), &'static str> {
        unsafe { formats::set_clipboard_files(paths) }.map_err(|_| "clipboard_files_write")
    }
    fn image(&mut self, image: PreparedImage) -> Result<Option<String>, &'static str> {
        unsafe {
            formats::set_clipboard_image_with_formats(
                formats::ImageData {
                    width: image.width,
                    height: image.height,
                    bytes: image.rgba,
                },
                image.animated.as_deref(),
                image.png.as_deref(),
            )
        }
        .map_err(|_| "clipboard_image_write")
    }
    fn rich(
        &mut self,
        text: &str,
        html: &str,
        named: &[NamedClipboardFormat],
        append: bool,
    ) -> Result<(), &'static str> {
        unsafe {
            if append {
                formats::append_clipboard_text_and_html(text, html)
            } else {
                formats::set_clipboard_text_and_html(text, html)
            }
            .map_err(|_| "clipboard_rich_write")?;
            formats::append_named_clipboard_formats(named).map_err(|_| "clipboard_named_write")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::ClipboardData;
    use crate::processing::discover;
    use crate::utils::{attach_rich_image_fallback, attach_rich_named_formats};

    #[derive(Default)]
    struct FakeSink {
        calls: Vec<&'static str>,
        text: String,
        html: String,
        named: Vec<(String, Vec<u8>)>,
        append: bool,
        fail_rich: bool,
        image: Option<PreparedImage>,
        paths: Vec<String>,
    }
    impl Sink for FakeSink {
        fn text(&mut self, text: &str) -> Result<(), &'static str> {
            self.calls.push("text");
            self.text = text.into();
            Ok(())
        }
        fn files(&mut self, paths: Vec<String>) -> Result<(), &'static str> {
            self.calls.push("files");
            self.paths = paths;
            Ok(())
        }
        fn image(&mut self, image: PreparedImage) -> Result<Option<String>, &'static str> {
            self.calls.push("image");
            self.image = Some(image);
            Ok(None)
        }
        fn rich(
            &mut self,
            text: &str,
            html: &str,
            named: &[NamedClipboardFormat],
            append: bool,
        ) -> Result<(), &'static str> {
            self.calls.push("rich");
            self.text = text.into();
            self.html = html.into();
            self.named = named
                .iter()
                .map(|format| (format.name.clone(), format.data.clone()))
                .collect();
            self.append = append;
            if self.fail_rich {
                Err("fixture_rich_failure")
            } else {
                Ok(())
            }
        }
    }
    fn entry(data: ClipboardData) -> ClipboardEntry {
        discover(&data, "fixture", None, 1)
    }
    fn encoded(bytes: &[u8], mime: &str) -> String {
        format!("data:image/{mime};base64,{}", STANDARD.encode(bytes))
    }
    fn png() -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 1, image::Rgba([17, 33, 49, 255]))
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        output.into_inner()
    }
    fn fragment(html: &str) -> &str {
        let offset = |key: &str| {
            html.lines()
                .find_map(|line| line.strip_prefix(key))
                .unwrap()
                .parse::<usize>()
                .unwrap()
        };
        &html[offset("StartFragment:")..offset("EndFragment:")]
    }
    #[test]
    fn text_and_plain_rich_preserve_exact_whitespace_without_html() {
        let body = "  中文\r\n\t ";
        for (data, plain) in [
            (ClipboardData::Text(body.into()), false),
            (
                ClipboardData::RichText {
                    text: body.into(),
                    html: "<b>private</b>".into(),
                },
                true,
            ),
        ] {
            let mut sink = FakeSink::default();
            let mut stored = entry(data);
            stored.content = body.into(); // Exercise the writer, not discovery normalization.
            PreparedWrite::prepare(&stored, plain, &[])
                .unwrap()
                .write_to(&mut sink)
                .unwrap();
            assert_eq!(sink.calls, ["text"]);
            assert_eq!(sink.text, body);
            assert!(sink.html.is_empty());
        }
    }
    #[test]
    fn cf_html_offsets_are_utf8_bytes_for_fragments_and_full_documents() {
        for html in [
            "<b>中文😀</b>",
            "<html><body><table><tr><td>中文😀</td></tr></table></body></html>",
        ] {
            let data = ClipboardData::RichText {
                text: "中文😀".into(),
                html: html.into(),
            };
            let mut sink = FakeSink::default();
            PreparedWrite::prepare(&entry(data), false, &[])
                .unwrap()
                .write_to(&mut sink)
                .unwrap();
            assert_eq!(sink.calls, ["rich"]);
            assert!(fragment(&sink.html).contains("中文😀"));
            let end_html: usize = sink
                .html
                .lines()
                .find_map(|line| line.strip_prefix("EndHTML:"))
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(end_html, sink.html.len());
        }
    }
    #[test]
    fn rich_named_office_formats_survive_without_metadata_in_cf_html() {
        let formats = [
            NamedClipboardFormat {
                name: "Rich Text Format".into(),
                data: b"{\\rtf1 fixture}".to_vec(),
            },
            NamedClipboardFormat {
                name: "Biff8".into(),
                data: vec![0, 9, 255, 7],
            },
        ];
        let html = attach_rich_named_formats("<table><tr><td>单元格</td></tr></table>", &formats);
        let mut sink = FakeSink::default();
        PreparedWrite::prepare(
            &entry(ClipboardData::RichText {
                text: "单元格".into(),
                html,
            }),
            false,
            &[],
        )
        .unwrap()
        .write_to(&mut sink)
        .unwrap();
        assert_eq!(sink.named.len(), 2);
        assert_eq!(sink.named[1], ("Biff8".into(), vec![0, 9, 255, 7]));
        assert!(!sink.html.contains("data_base64"));
        assert!(fragment(&sink.html).contains("<td>单元格</td>"));
    }
    #[test]
    fn preview_fallback_never_replaces_pasteable_html_or_text() {
        let html = attach_rich_image_fallback("<b>body</b>", "data:image/png;base64,invalid");
        let mut sink = FakeSink::default();
        PreparedWrite::prepare(
            &entry(ClipboardData::RichText {
                text: "body".into(),
                html,
            }),
            false,
            &[],
        )
        .unwrap()
        .write_to(&mut sink)
        .unwrap();
        assert_eq!(sink.calls, ["rich"]);
        assert!(sink.image.is_none());
        assert!(!sink.append);
        assert_eq!(fragment(&sink.html), "<b>body</b>");
    }
    #[test]
    fn image_only_rich_fallback_appends_html_without_clearing_static_raster() {
        let html = attach_rich_image_fallback("", &encoded(&png(), "png"));
        let mut sink = FakeSink::default();
        let prepared = PreparedWrite::prepare(
            &entry(ClipboardData::RichText {
                text: "".into(),
                html,
            }),
            false,
            &[],
        )
        .unwrap();
        assert!(prepared.image_visual_hash().is_some());
        prepared.write_to(&mut sink).unwrap();
        assert_eq!(sink.calls, ["image", "rich"]);
        assert!(sink.image.is_some());
        assert!(sink.append); // Still images have no temporary CF_HDROP path.
    }
    #[test]
    fn decoded_rgba_has_an_explicit_allocation_bound() {
        assert!(validate_dimensions(8192, 4096).is_ok());
        for (width, height) in [(0, 1), (16385, 1), (16384, 16384), (u32::MAX, u32::MAX)] {
            assert_eq!(validate_dimensions(width, height), Err("image_dimensions"));
        }
    }
    #[test]
    fn static_images_prepare_rgba_and_png_compatibility() {
        let mut sink = FakeSink::default();
        PreparedWrite::prepare(
            &entry(ClipboardData::Image {
                data_url: encoded(&png(), "png"),
            }),
            false,
            &[],
        )
        .unwrap()
        .write_to(&mut sink)
        .unwrap();
        let image = sink.image.unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.rgba, vec![17, 33, 49, 255, 17, 33, 49, 255]);
        assert!(image.png.unwrap().starts_with(b"\x89PNG"));
        assert!(image.animated.is_none());
    }
    #[test]
    fn animated_gif_keeps_original_bytes_and_omits_static_png() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for value in [10, 220] {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        2,
                        1,
                        image::Rgba([value, 0, 0, 255]),
                    )))
                    .unwrap();
            }
        }
        let mut sink = FakeSink::default();
        PreparedWrite::prepare(
            &entry(ClipboardData::Image {
                data_url: encoded(&bytes, "gif"),
            }),
            false,
            &[],
        )
        .unwrap()
        .write_to(&mut sink)
        .unwrap();
        let image = sink.image.unwrap();
        assert_eq!(image.animated.unwrap(), bytes);
        assert!(image.png.is_none());
    }
    #[test]
    fn file_lists_keep_unicode_and_whitespace_and_reject_bad_paths() {
        let files = vec![
            "C:\\fixture\\文件.txt".into(),
            "C:\\fixture\\ spaced name .txt".into(),
        ];
        let mut sink = FakeSink::default();
        PreparedWrite::prepare(&entry(ClipboardData::Files(files.clone())), false, &[])
            .unwrap()
            .write_to(&mut sink)
            .unwrap();
        assert_eq!(sink.paths, files);
        for content in ["", "relative.txt", "C:\\bad\0.txt"] {
            let mut e = entry(ClipboardData::Text(content.into()));
            e.content_type = "file".into();
            assert!(PreparedWrite::prepare(&e, false, &[]).is_err());
        }
    }
    #[test]
    fn preparation_rejects_invalid_images_and_nul_before_any_write() {
        for data in [
            ClipboardData::Text("bad\0text".into()),
            ClipboardData::Image {
                data_url: "data:image/png;base64,bad!".into(),
            },
            ClipboardData::Image {
                data_url: encoded(b"not a png", "png"),
            },
        ] {
            assert!(PreparedWrite::prepare(&entry(data), false, &[]).is_err());
        }
    }
    #[test]
    fn local_images_require_explicit_roots_and_rich_failure_is_propagated() {
        let directory = std::env::temp_dir().join(format!("TieZ-write-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("原图%20 fixture.png");
        std::fs::write(&path, png()).unwrap();
        let e = entry(ClipboardData::Image {
            data_url: path.to_string_lossy().into_owned(),
        });
        assert!(PreparedWrite::prepare(&e, false, &[]).is_err());
        assert!(PreparedWrite::prepare(&e, false, &[directory.clone()]).is_ok());
        let uri = format!(
            "file:///{}",
            path.to_string_lossy()
                .replace('\\', "/")
                .replace('%', "%25")
                .replace(' ', "%20")
        );
        let uri_entry = entry(ClipboardData::Image { data_url: uri });
        assert!(PreparedWrite::prepare(&uri_entry, false, &[directory.clone()]).is_ok());
        let html = attach_rich_image_fallback("", &encoded(&png(), "png"));
        let mut sink = FakeSink {
            fail_rich: true,
            ..Default::default()
        };
        let result = PreparedWrite::prepare(
            &entry(ClipboardData::RichText {
                text: "".into(),
                html,
            }),
            false,
            &[],
        )
        .unwrap()
        .write_to(&mut sink);
        assert_eq!(result, Err("fixture_rich_failure"));
        assert_eq!(sink.calls, ["image", "rich"]);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
