#[cfg(feature = "agent")]
pub mod watcher;

/// Best-effort classification of clipboard text, used purely for display
/// (e.g. rendering a link with an Open button). Never used to decide
/// whether to execute or fetch anything automatically -- see section 8,
/// "do not execute clipboard contents".
pub fn detect_content_type(content: &str) -> &'static str {
    let trimmed = content.trim();

    if is_url(trimmed) {
        return "url";
    }
    if looks_like_code(trimmed) {
        return "code";
    }
    "text"
}

fn is_url(s: &str) -> bool {
    if s.contains(char::is_whitespace) {
        return false;
    }
    s.starts_with("http://") || s.starts_with("https://")
}

fn looks_like_code(s: &str) -> bool {
    let code_markers = ["{", "}", ";", "=>", "function ", "fn ", "def ", "const ", "import "];
    let has_marker = code_markers.iter().any(|m| s.contains(m));
    let multiline = s.lines().count() > 1;
    has_marker && (multiline || s.len() < 400)
}

/// Cap clipboard entries to a sane size so one huge paste can't blow out
/// storage or the WebSocket fan-out.
pub const MAX_CLIPBOARD_BYTES: usize = 256 * 1024; // 256 KiB

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_urls() {
        assert_eq!(detect_content_type("https://github.com/example"), "url");
        assert_eq!(detect_content_type("  http://192.168.1.1:8787 "), "url");
    }

    #[test]
    fn detects_plain_text() {
        assert_eq!(detect_content_type("Hello from my PC!"), "text");
    }

    #[test]
    fn detects_code() {
        assert_eq!(detect_content_type("fn main() { println!(\"hi\"); }"), "code");
    }
}
