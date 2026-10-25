use percent_encoding::percent_decode_str;
use reqwest::header::{HeaderMap, CONTENT_DISPOSITION};
use std::path::Path;

/// File name from Content-Disposition, falling back to the last URL path segment.
pub fn from_response(headers: &HeaderMap, url: &reqwest::Url) -> String {
    let from_header = headers
        .get(CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(from_disposition);
    let name = from_header.unwrap_or_else(|| {
        let last = url.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
        percent_decode_str(last).decode_utf8_lossy().into_owned()
    });
    sanitize(&name)
}

fn from_disposition(v: &str) -> Option<String> {
    let mut plain = None;
    for part in v.split(';').map(str::trim) {
        let Some((key, val)) = part.split_once('=') else { continue };
        match key.trim().to_ascii_lowercase().as_str() {
            // RFC 5987: filename*=UTF-8''na%20me.zip — preferred over plain filename.
            "filename*" => {
                let encoded = val.splitn(3, '\'').nth(2)?;
                return Some(percent_decode_str(encoded).decode_utf8_lossy().into_owned());
            }
            "filename" => plain = Some(val.trim().trim_matches('"').to_string()),
            _ => {}
        }
    }
    plain
}

pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']);
    if cleaned.is_empty() { "download".into() } else { cleaned.into() }
}

/// `name`, or `stem (n).ext` if that file (or its partial download) already exists.
pub fn unique(dir: &Path, name: &str) -> String {
    let taken = |n: &str| dir.join(n).exists() || dir.join(format!("{n}.part")).exists();
    if !taken(name) {
        return name.into();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (1..).map(|i| format!("{stem} ({i}){ext}")).find(|n| !taken(n)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_variants() {
        assert_eq!(from_disposition(r#"attachment; filename="a b.zip""#).unwrap(), "a b.zip");
        assert_eq!(
            from_disposition("attachment; filename=x.bin; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf").unwrap(),
            "résumé.pdf"
        );
        assert_eq!(from_disposition("inline"), None);
    }

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize("a/b:c?.txt"), "a_b_c_.txt");
        assert_eq!(sanitize("  "), "download");
    }
}
