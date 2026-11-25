//! Local path normalisation shared by the shim wrapper and the session layer (spec §4.4; the
//! F-09 follow-up in the audit).
//!
//! Rust canonicalises a media path and passes `file:<path>` to libav. On Windows `canonicalize`
//! returns verbatim paths (`\\?\C:\...`, `\\?\UNC\server\share\...`), which FFmpeg must not see as
//! such: a verbatim prefix is not a drive path, and the `UNC\` form left by simply stripping
//! `\\?\` is a *relative* path. Network shares on mapped drives are returned as `\\?\UNC\...` too,
//! so getting this wrong silently breaks NAS media while the import check (subprocess) still
//! succeeds.

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
}
