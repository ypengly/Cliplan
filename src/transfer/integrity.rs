use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};

/// Reduce a client-supplied filename to a single safe path component.
///
/// This is a defense-in-depth measure: the file is *never* written to a
/// path derived directly from user input anyway (storage uses a random
/// UUID filename, see `upload.rs`), but the sanitized name is still shown
/// back to users and used for the `Content-Disposition` download header, so
/// it must not be able to inject path separators, traversal sequences, or
/// control characters.
pub fn sanitize_filename(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file");

    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || c == '\0' {
                '_'
            } else {
                c
            }
        })
        .collect();

    let cleaned = cleaned.trim();
    let cleaned = cleaned.trim_start_matches('.'); // no dotfiles / ".."

    let result = if cleaned.is_empty() { "file".to_string() } else { cleaned.to_string() };

    // Cap length to something reasonable for filesystems / headers.
    if result.len() > 255 {
        result.chars().take(255).collect()
    } else {
        result
    }
}

/// Stream-hash a file already written to disk. Used both to compute the
/// hash of a freshly-uploaded file for verification, and to let a client
/// verify a downloaded file matches.
pub async fn hash_file(path: &Path) -> std::io::Result<String> {
    let file = File::open(path).await?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_path_traversal() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("..\\..\\secret.txt"), "secret.txt");
        assert_eq!(sanitize_filename("/etc/passwd"), "passwd");
    }

    #[test]
    fn strips_leading_dots_and_control_chars() {
        assert_eq!(sanitize_filename("...hidden"), "hidden");
        assert_eq!(sanitize_filename("bad\x00name.txt"), "bad_name.txt");
    }

    #[test]
    fn falls_back_to_default_for_empty() {
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_filename("..."), "file");
    }

    #[test]
    fn keeps_normal_names_untouched() {
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("My Report (final).pdf"), "My Report (final).pdf");
    }
}
