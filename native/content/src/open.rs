//! Prepare external-open content without launching apps, writing files or changing history.
//! The core must bind the authoritative entry, history revision and sensitive consent.
use crate::domain::models::ClipboardEntry;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};

// No Debug/Serialize implementations: paths, URLs and export bytes can be private.
#[derive(PartialEq, Eq)]
pub enum Handler {
    System,
    Desktop(PathBuf),
    Packaged(String),
}
pub enum Target {
    Url(String),
    Existing(PathBuf),
    Export(Export),
}
pub struct Export {
    extension: &'static str,
    bytes: Vec<u8>,
    watch: bool,
}
impl Export {
    pub fn extension(&self) -> &'static str {
        self.extension
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn watch(&self) -> bool {
        self.watch
    }
}
pub struct PreparedOpen {
    pub handler: Handler,
    pub target: Target,
}
fn configured_handler(
    kind: &str,
    values: &BTreeMap<String, String>,
) -> Result<Handler, &'static str> {
    let nonblank = |key: &str| values.get(key).filter(|value| !value.trim().is_empty());
    let selected = nonblank(&format!("app.{kind}"))
        .or_else(|| (kind == "code").then(|| nonblank("app.text")).flatten());
    let Some(value) = selected else {
        return Ok(Handler::System);
    };
    if value.chars().any(char::is_control) {
        return Err("open_handler");
    }
    // Preserve the legacy default-handler fallback for non-Windows app selections.
    if value.starts_with("/Applications/") || value.ends_with(".app") {
        return Ok(Handler::System);
    }
    if Path::new(value).is_absolute() {
        return Ok(Handler::Desktop(value.into()));
    }
    // A packaged-app identity is data, never an interpolated shell command.
    if value.len() <= 255
        && value.split_once('!').is_some_and(|(family, app)| {
            !family.is_empty()
                && !app.is_empty()
                && !app.contains('!')
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.!-".contains(&byte))
        })
    {
        return Ok(Handler::Packaged(value.clone()));
    }
    Err("open_handler")
}
pub fn normalize_external_url(content: &str) -> Result<String, &'static str> {
    let value = content.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err("open_url");
    }
    let value = if value.to_ascii_lowercase().starts_with("www.") {
        format!("https://{value}")
    } else {
        value.into()
    };
    let url = reqwest::Url::parse(&value).map_err(|_| "open_url")?;
    match url.scheme() {
        "http" | "https" | "ftp" | "ftps" if url.host_str().is_some() => {}
        "mailto" if !url.path().is_empty() => {}
        _ => return Err("open_url"),
    }
    Ok(url.to_string())
}
fn export(bytes: Vec<u8>, extension: &'static str, watch: bool) -> Target {
    Target::Export(Export {
        bytes,
        extension,
        watch,
    })
}
impl PreparedOpen {
    pub fn prepare(
        entry: &ClipboardEntry,
        values: &BTreeMap<String, String>,
        image_roots: &[PathBuf],
    ) -> Result<Self, &'static str> {
        let kind = entry.content_type.as_str();
        if !matches!(
            kind,
            "text" | "code" | "url" | "link" | "rich_text" | "image" | "file" | "video"
        ) {
            return Err("open_type");
        }
        let handler = configured_handler(kind, values)?;
        let target = match kind {
            "url" | "link" => Target::Url(normalize_external_url(&entry.content)?),
            "file" | "video" => {
                // Legacy open_content opens the first path only; missing paths become a text document.
                let first = Path::new(entry.content.lines().next().unwrap_or("").trim());
                if first.is_absolute() && first.exists() {
                    Target::Existing(first.into())
                } else {
                    export(entry.content.as_bytes().to_vec(), "txt", true)
                }
            }
            "image" => {
                let bytes = crate::write::image_bytes(&entry.content, image_roots)?;
                let format = image::guess_format(&bytes).map_err(|_| "image_decode")?;
                let extension = match format {
                    image::ImageFormat::Png => "png",
                    image::ImageFormat::Jpeg => "jpg",
                    image::ImageFormat::Bmp => "bmp",
                    image::ImageFormat::Gif => "gif",
                    image::ImageFormat::WebP => "webp",
                    _ => return Err("image_decode"),
                };
                let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(16_384);
                limits.max_image_height = Some(16_384);
                limits.max_alloc = Some(128 * 1024 * 1024);
                reader.limits(limits);
                let decoded = reader.decode().map_err(|_| "image_decode")?;
                crate::write::validate_dimensions(decoded.width(), decoded.height())?;
                // Export original bytes, including animation; never flatten or launch an image using an arbitrary source suffix.
                export(bytes, extension, true)
            }
            "rich_text" => match entry
                .html_content
                .as_deref()
                .filter(|html| !html.trim().is_empty())
            {
                Some(html) => export(html.as_bytes().to_vec(), "html", false),
                None => export(entry.content.as_bytes().to_vec(), "txt", false),
            },
            _ => export(entry.content.as_bytes().to_vec(), "txt", true),
        };
        Ok(Self { handler, target })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use image::{DynamicImage, ImageFormat};
    fn entry(kind: &str, content: &str) -> ClipboardEntry {
        ClipboardEntry {
            id: -7,
            content_type: kind.into(),
            content: content.into(),
            html_content: None,
            source_app: "Fixture".into(),
            source_app_path: None,
            timestamp: 1,
            is_pinned: false,
            tags: vec![],
            use_count: 0,
            is_external: false,
            pinned_order: 0,
            preview: "fixture".into(),
            file_preview_exists: true,
        }
    }
    fn prepared(kind: &str, body: &str) -> PreparedOpen {
        PreparedOpen::prepare(&entry(kind, body), &BTreeMap::new(), &[]).unwrap()
    }
    fn exported(plan: &PreparedOpen) -> &Export {
        match &plan.target {
            Target::Export(value) => value,
            _ => panic!("Expected export"),
        }
    }
    #[test]
    fn text_and_code_preserve_whitespace_crlf_and_source() {
        for kind in ["text", "code"] {
            let body = "  中文\r\n\t ";
            let source = entry(kind, body);
            let plan = PreparedOpen::prepare(&source, &BTreeMap::new(), &[]).unwrap();
            let output = exported(&plan);
            assert!(plan.handler == Handler::System);
            assert_eq!(output.bytes(), body.as_bytes());
            assert_eq!(output.extension(), "txt");
            assert!(output.watch());
            assert_eq!(source.content, body);
            assert_eq!((source.id, source.use_count), (-7, 0));
        }
    }
    #[test]
    fn handlers_preserve_exact_paths_code_fallback_packaged_identity_and_defaults() {
        let mut settings = BTreeMap::from([(
            "app.text".into(),
            r"C:\Program Files\编辑器\editor.exe".into(),
        )]);
        let source = entry("code", "body");
        assert!(
            matches!(PreparedOpen::prepare(&source, &settings, &[]).unwrap().handler, Handler::Desktop(path) if path == Path::new(&settings["app.text"]))
        );
        settings.insert("app.code".into(), "Family_123!App.Id".into());
        assert!(
            matches!(PreparedOpen::prepare(&source, &settings, &[]).unwrap().handler, Handler::Packaged(value) if value == "Family_123!App.Id")
        );
        settings.insert("app.code".into(), " \t".into());
        assert!(matches!(
            PreparedOpen::prepare(&source, &settings, &[])
                .unwrap()
                .handler,
            Handler::Desktop(_)
        ));
        settings.insert("app.code".into(), "/Applications/Editor.app".into());
        assert!(
            PreparedOpen::prepare(&source, &settings, &[])
                .unwrap()
                .handler
                == Handler::System
        );
        assert_eq!(settings["app.code"], "/Applications/Editor.app");
    }
    #[test]
    fn invalid_handlers_are_rejected_as_data_without_shell_fallback() {
        for handler in [
            "Family!App'; Start-Process calc",
            "Family!App!Other",
            "!App",
            "Family!",
            "editor.exe --arg",
            concat!(r"C:\editor.exe", "\0"),
            "Family!App\n",
        ] {
            let values = BTreeMap::from([("app.text".into(), handler.into())]);
            assert!(matches!(
                PreparedOpen::prepare(&entry("text", "body"), &values, &[]),
                Err("open_handler")
            ));
        }
    }
    #[test]
    fn urls_match_legacy_protocols_preserve_queries_and_do_not_modify_source() {
        for (body, expected) in [
            (
                "https://example.test/?x=1&ver",
                "https://example.test/?x=1&ver",
            ),
            (
                " www.example.test/path?a=1&b=2 ",
                "https://www.example.test/path?a=1&b=2",
            ),
            ("ftp://example.test/file", "ftp://example.test/file"),
            ("ftps://example.test/file", "ftps://example.test/file"),
            ("mailto:user@example.test", "mailto:user@example.test"),
        ] {
            for kind in ["url", "link"] {
                let source = entry(kind, body);
                let plan = PreparedOpen::prepare(&source, &BTreeMap::new(), &[]).unwrap();
                assert!(matches!(plan.target, Target::Url(value) if value == expected));
                assert_eq!(source.content, body);
            }
        }
        for value in [
            "file:///C:/Windows/cmd.exe",
            "javascript:alert(1)",
            "data:text/html,hello",
            "ms-msdt:/id PCWDiagnostic",
            r"C:\Windows\cmd.exe",
            "https://example.test/\n&ver",
            "https://example.test/\0",
            "mailto:",
            "",
        ] {
            assert_eq!(normalize_external_url(value).unwrap_err(), "open_url");
        }
    }
    #[test]
    fn rich_html_and_plain_fallback_keep_original_bytes_without_legacy_watch() {
        let mut source = entry("rich_text", "  fallback\r\n ");
        for html in [
            Some("<html><body><b>中文</b></body></html>"),
            None,
            Some(" \r\n"),
        ] {
            source.html_content = html.map(str::to_string);
            let plan = PreparedOpen::prepare(&source, &BTreeMap::new(), &[]).unwrap();
            let output = exported(&plan);
            let has_html = html.is_some_and(|value| !value.trim().is_empty());
            assert_eq!(output.extension(), if has_html { "html" } else { "txt" });
            assert_eq!(
                output.bytes(),
                if has_html {
                    html.unwrap().as_bytes()
                } else {
                    source.content.as_bytes()
                }
            );
            assert!(!output.watch());
            assert_eq!(source.html_content.as_deref(), html);
        }
    }
    #[test]
    fn file_video_open_only_first_existing_path_or_export_original_path_list() {
        let directory = std::env::temp_dir().join(format!("TieZ-open-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let first = directory.join("文件%20 ' & one.txt");
        let second = directory.join("第二项.txt");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();
        for kind in ["file", "video"] {
            let content = format!(" {}\r\n{}", first.display(), second.display());
            assert!(
                matches!(prepared(kind, &content).target, Target::Existing(path) if path == first)
            );
            let missing = format!(
                "{}\n{}",
                directory.join("missing.txt").display(),
                second.display()
            );
            let plan = prepared(kind, &missing);
            assert_eq!(exported(&plan).bytes(), missing.as_bytes());
            assert!(exported(&plan).watch());
        }
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn image_exports_preserve_exact_supported_encoded_formats_and_animation() {
        let image = DynamicImage::new_rgba8(2, 3);
        for (format, extension) in [
            (ImageFormat::Png, "png"),
            (ImageFormat::Bmp, "bmp"),
            (ImageFormat::Gif, "gif"),
            (ImageFormat::WebP, "webp"),
        ] {
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, format).unwrap();
            let original = bytes.into_inner();
            let body = format!("data:image/ignored;base64,{}", STANDARD.encode(&original));
            let plan = prepared("image", &body);
            let output = exported(&plan);
            assert_eq!(output.bytes(), original);
            assert_eq!(output.extension(), extension);
            assert!(output.watch());
        }
        let mut jpeg = Cursor::new(Vec::new());
        image
            .to_rgb8()
            .write_to(&mut jpeg, ImageFormat::Jpeg)
            .unwrap();
        let original = jpeg.into_inner();
        let plan = prepared(
            "image",
            &format!("data:image/jpeg;base64,{}", STANDARD.encode(&original)),
        );
        assert_eq!(exported(&plan).bytes(), original);
        assert_eq!(exported(&plan).extension(), "jpg");
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            encoder
                .encode_frame(image::Frame::new(image.to_rgba8()))
                .unwrap();
            encoder
                .encode_frame(image::Frame::new(image.to_rgba8()))
                .unwrap();
        }
        let plan = prepared(
            "image",
            &format!("data:image/gif;base64,{}", STANDARD.encode(&gif)),
        );
        assert_eq!(exported(&plan).bytes(), gif);
    }
    #[test]
    fn image_paths_are_scoped_readonly_and_bad_data_has_no_artifact_side_effects() {
        let directory =
            std::env::temp_dir().join(format!("TieZ-open-image-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("原图.png");
        DynamicImage::new_rgba8(2, 3).save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        let source = entry("image", path.to_str().unwrap());
        assert!(PreparedOpen::prepare(&source, &BTreeMap::new(), &[]).is_err());
        let plan = PreparedOpen::prepare(&source, &BTreeMap::new(), &[directory.clone()]).unwrap();
        assert_eq!(exported(&plan).bytes(), original);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        for content in [
            "data:image/png;base64,bad!",
            "data:image/png;base64,AAAA",
            "relative.png",
        ] {
            assert!(PreparedOpen::prepare(
                &entry("image", content),
                &BTreeMap::new(),
                &[directory.clone()]
            )
            .is_err());
        }
        assert!(matches!(
            PreparedOpen::prepare(&entry("unknown", "body"), &BTreeMap::new(), &[]),
            Err("open_type")
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
