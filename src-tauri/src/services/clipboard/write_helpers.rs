//! Shared production clipboard serialization rules. No host state or OS writes.
use regex::Regex;
use std::sync::OnceLock;

pub(crate) fn generate_cf_html(html: &str) -> String {
    static BODY_OPEN_RE: OnceLock<Regex> = OnceLock::new();
    static BODY_CLOSE_RE: OnceLock<Regex> = OnceLock::new();
    static HTML_TAG_RE: OnceLock<Regex> = OnceLock::new();

    let body_open_re = BODY_OPEN_RE.get_or_init(|| Regex::new(r"(?is)<body\b[^>]*>").unwrap());
    let body_close_re = BODY_CLOSE_RE.get_or_init(|| Regex::new(r"(?is)</body\s*>").unwrap());
    let html_tag_re = HTML_TAG_RE.get_or_init(|| Regex::new(r"(?is)<html\b").unwrap());

    let wrap_with_body = |fragment: &str| {
        format!(
            "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n</head>\n<body>\n<!--StartFragment-->{}<!--EndFragment-->\n</body>\n</html>",
            fragment
        )
    };

    let mut html_content = super::utils::repair_html_fragment(html);
    let has_html_tag = html_tag_re.is_match(&html_content);
    let has_start = html_content.contains("<!--StartFragment-->");
    let has_end = html_content.contains("<!--EndFragment-->");

    if !has_html_tag {
        html_content = wrap_with_body(&html_content);
    } else if !(has_start && has_end) {
        if let Some(open_match) = body_open_re.find(&html_content) {
            let open_end = open_match.end();

            if !has_end {
                if let Some(close_match) = body_close_re.find(&html_content) {
                    if close_match.start() >= open_end {
                        html_content.insert_str(close_match.start(), "<!--EndFragment-->");
                    } else {
                        html_content.push_str("<!--EndFragment-->");
                    }
                } else {
                    html_content.push_str("<!--EndFragment-->");
                }
            }

            if !has_start {
                html_content.insert_str(open_end, "<!--StartFragment-->");
            }
        } else {
            html_content = wrap_with_body(&html_content);
        }
    }

    if !(html_content.contains("<!--StartFragment-->")
        && html_content.contains("<!--EndFragment-->"))
    {
        html_content = wrap_with_body(&html_content);
    }

    let header_template = "Version:1.0\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let header_len = header_template.len();

    let start_html = header_len;
    let end_html = start_html + html_content.len();
    let start_fragment = start_html
        + html_content.find("<!--StartFragment-->").unwrap_or(0)
        + "<!--StartFragment-->".len();
    let end_fragment = start_html
        + html_content
            .find("<!--EndFragment-->")
            .unwrap_or(html_content.len());

    let header = format!(
        "Version:1.0\r\nStartHTML:{:0>10}\r\nEndHTML:{:0>10}\r\nStartFragment:{:0>10}\r\nEndFragment:{:0>10}\r\n",
        start_html,
        end_html,
        start_fragment,
        end_fragment
    );
    format!("{}{}", header, html_content)
}
pub(crate) fn is_gif_image_bytes(bytes: &[u8]) -> bool {
    bytes.len() > 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))
}

pub(crate) fn is_animated_clipboard_image_bytes(bytes: &[u8]) -> bool {
    is_gif_image_bytes(bytes) || super::utils::is_animated_webp_payload(bytes)
}

/// Decide whether the PNG clipboard format should accompany an image paste.
///
/// Many Windows apps prefer `PNG` / `image/png` over GIF/WebP named formats and
/// CF_HDROP. Attaching a first-frame PNG beside an animated image therefore
/// makes paste targets materialize a static PNG even when the history entry
/// still has the original animation.
pub(crate) fn should_attach_png_clipboard_format(is_animated: bool) -> bool {
    !is_animated
}
