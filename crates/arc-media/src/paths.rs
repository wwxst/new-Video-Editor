//! Local path normalisation shared by the shim wrapper and the session layer (spec §4.4; the
//! F-09 follow-up in the audit).
//!
//! Rust canonicalises a media path and passes `file:<path>` to libav. On Windows `canonicalize`
//! returns verbatim paths (`\\?\C:\...`, `\\?\UNC\server\share\...`), which FFmpeg must not see as
//! such: a verbatim prefix is not a drive path, and the `UNC\` form left by simply stripping
//! `\\?\` is a *relative* path. Network shares on mapped drives are returned as `\\?\UNC\...` too,
//! so getting this wrong silently breaks NAS media while the import check (subprocess) still
//! succeeds.

use std::path::Path;

/// Map a canonical Windows path to the plain form:
/// - `\\?\UNC\server\share\x` → `\\server\share\x`
/// - `\\?\C:\dir\x` → `C:\dir\x`
/// - anything else is unchanged.
pub fn normalize_verbatim(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

/// True when `s` is an absolute local path FFmpeg may open: a drive path (`C:\`, `C:/`) or a UNC
/// share (`\\server\share`). Verbatim prefixes have already been normalised away.
pub fn is_local_absolute(s: &str) -> bool {
    if cfg!(windows) {
        let bytes = s.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/');
        drive || s.starts_with(r"\\")
    } else {
        s.starts_with('/')
    }
}

/// `file:<absolute path>` for an existing local file; `None` for anything else (missing, relative,
/// URL, device namespace). This is the only form the shim and the CLI tools are given (spec §4.4).
pub fn file_url(path: &Path) -> Option<String> {
    let canon = std::fs::canonicalize(path).ok()?;
    let s = normalize_verbatim(&canon.to_string_lossy());
    is_local_absolute(&s).then(|| format!("file:{s}"))
}

/// `file:` URL for a path that may not exist yet but whose parent does (encoder output).
pub fn output_file_url(path: &Path) -> Option<String> {
    if let Some(url) = file_url(path) {
        return Some(url);
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty())?;
    let canon = std::fs::canonicalize(parent).ok()?;
    let mut full = canon;
    full.push(path.file_name()?);
    let s = normalize_verbatim(&full.to_string_lossy());
    is_local_absolute(&s).then(|| format!("file:{s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_prefixes_keep_drive_and_unc_paths_local() {
        assert_eq!(
            normalize_verbatim(r"\\?\UNC\server\share\a.mkv"),
            r"\\server\share\a.mkv"
        );
        assert_eq!(
            normalize_verbatim(r"\\?\C:\media\a b.mkv"),
            r"C:\media\a b.mkv"
        );
        assert_eq!(normalize_verbatim(r"C:\media\a.mkv"), r"C:\media\a.mkv");
        assert_eq!(
            normalize_verbatim(r"\\server\share\a.mkv"),
            r"\\server\share\a.mkv"
        );
        assert_eq!(normalize_verbatim("/home/u/a.mkv"), "/home/u/a.mkv");
    }

    #[cfg(windows)]
    #[test]
    fn local_absolute_accepts_drive_and_unc_only() {
        assert!(is_local_absolute(r"C:\media\a.mkv"));
        assert!(is_local_absolute("C:/media/a.mkv"));
        assert!(is_local_absolute(r"\\server\share\a.mkv"));
        assert!(!is_local_absolute(r"UNC\server\share\a.mkv"));
        assert!(!is_local_absolute(r"media\a.mkv"));
        assert!(!is_local_absolute(&normalize_verbatim(
            r"\\?\Volume{0000}\a.mkv"
        )));
    }

    #[test]
    fn file_url_requires_an_existing_local_file() {
        assert_eq!(file_url(Path::new("http://127.0.0.1:9/x.mp4")), None);
        assert_eq!(file_url(Path::new(r"C:\does\not\exist\x.mkv")), None);
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("a b.mkv");
        std::fs::write(&p, b"x").expect("write");
        let url = file_url(&p).expect("local file");
        assert!(url.starts_with("file:"), "{url}");
        assert!(url.ends_with("a b.mkv"), "{url}");

        let out = dir.path().join("new output.mkv");
        let url = output_file_url(&out).expect("output in an existing directory");
        assert!(url.starts_with("file:") && url.ends_with("new output.mkv"), "{url}");
    }
}
