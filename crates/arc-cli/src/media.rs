//! Session-layer media resolution (docs/06 §4.1): every file-referencing command from a client is
//! resolved, checked and probed by the session before it reaches the engine. Document asset paths
//! are untrusted input; libav only ever opens local `file:` URLs (audit F-09).

use arc_core::model::{AssetKind, AssetStatus};
use arc_core::{Command, CommandError, ErrorKind};
use std::path::{Path, PathBuf};

/// True when `s` starts with a URL/protocol scheme (`http:`, `concat:`, `handle:`, `file:`, ...).
/// A Windows drive letter (`C:\...`, `C:/...`) is a path, not a scheme.
pub fn has_url_scheme(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    if scheme.is_empty() {
        return false;
    }
    if scheme.len() == 1 && scheme.as_bytes()[0].is_ascii_alphabetic() {
        return !(rest.starts_with('\\') || rest.starts_with('/'));
    }
    let mut it = scheme.chars();
    let Some(first) = it.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && it.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Store a media path relative to the project folder when the media lives below it. The stored
/// form uses forward slashes and never keeps a Windows verbatim prefix (audit F-09 follow-up).
pub fn project_relative(project: &Path, media: &Path) -> String {
    let abs = std::fs::canonicalize(media).unwrap_or_else(|_| media.to_path_buf());
    let root = project.parent().and_then(|p| std::fs::canonicalize(p).ok());
    let rel = root
        .as_ref()
        .and_then(|r| abs.strip_prefix(r).ok())
        .map(Path::to_path_buf);
    let chosen = rel.unwrap_or(abs);
    arc_media::paths::normalize_verbatim(&chosen.to_string_lossy()).replace('\\', "/")
}

fn absolute(root: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Resolve every `asset.import` in `commands` against `project_path` (the container):
/// reject URL/protocol paths, run the subprocess import check, and replace the client-supplied
/// `kind`, `probe` and `status` with its result. Other commands are untouched.
pub fn resolve_imports(project_path: &Path, commands: &mut [Command]) -> Result<(), CommandError> {
    let root = project_path.parent().unwrap_or(Path::new("."));
    for (i, cmd) in commands.iter_mut().enumerate() {
        let Command::AssetImport {
            path,
            kind,
            probe,
            status,
            ..
        } = cmd
        else {
            continue;
        };
        if has_url_scheme(path) || is_device_path(path) {
            return Err(CommandError::new(
                ErrorKind::InvalidValue,
                format!(
                    "asset path must be a local file path, not a URL, protocol, device or \
                     verbatim path: {path}"
                ),
            )
            .at(format!("commands[{i}].path")));
        }
        let abs = absolute(root, path);
        let outcome = arc_media::probe::import_check(&abs);
        let stored = project_relative(project_path, &abs);
        match outcome {
            arc_media::probe::ImportOutcome::Ok { kind: k, probe: p } => {
                *kind = k;
                *probe = Some(p);
                *status = AssetStatus::Ok;
            }
            arc_media::probe::ImportOutcome::Quarantined {
                kind: k,
                probe: p,
                ..
            } => {
                *kind = k.unwrap_or(AssetKind::Video);
                *probe = p;
                *status = AssetStatus::Quarantined;
            }
            arc_media::probe::ImportOutcome::Offline { .. } => {
                *kind = AssetKind::Video;
                *probe = None;
                *status = AssetStatus::Offline;
            }
        }
        *path = stored;
    }
    Ok(())
}

    /// True when `s` uses a Windows device-namespace or verbatim prefix (`\\.\pipe\x`,
    /// `\\?\GLOBALROOT\...`, `//./...`, `//?/...`); these are handle-style accesses, not media files.
    pub fn is_device_path(s: &str) -> bool {
        let lower = s.to_ascii_lowercase();
        [
            r"\\.\", r"\\?\", r"\.\", r"\?\", "//./", "//?/",
        ]
        .iter()
        .any(|p| lower.starts_with(p))
    }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_detection_distinguishes_drive_letters() {
        for s in [
            "http://127.0.0.1:9/x.mp4",
            "https://example.com/x",
            "handle:1",
            "concat:a|b",
            "file:C:/x.mkv",
            "nested:thing",
        ] {
            assert!(has_url_scheme(s), "{s}");
        }
        for s in [
            r"C:\media\x.mkv",
            "C:/media/x.mkv",
            "media/x.mkv",
            "../x:y.mkv",
            "\\\\server\\share\\x.mkv",
        ] {
            assert!(!has_url_scheme(s), "{s}");
        }
    }

    #[test]
    fn device_namespaces_are_rejected_but_unc_is_not() {
        for s in [
            r"\\.\pipe\arcline",
            r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1\x.mp4",
            r"\.\pipe\x",
            r"\?\C:\x",
            "//./pipe/arcline",
            "//?/C:/x.mp4",
        ] {
            assert!(is_device_path(s), "{s}");
        }
        for s in [
            r"C:\media\x.mkv",
            "C:/media/x.mkv",
            r"\\server\share\x.mkv",
            "media/x.mkv",
        ] {
            assert!(!is_device_path(s), "{s}");
        }
    }

    #[test]
    fn relative_paths_stay_relative() {
        let dir = tempfile::tempdir().expect("tmp");
        let project = dir.path().join("p.arcline");
        let media = dir.path().join("media").join("a.mov");
        std::fs::create_dir_all(media.parent().expect("parent")).expect("mkdir");
        std::fs::write(&media, b"x").expect("write");
        assert_eq!(project_relative(&project, &media), "media/a.mov");
    }

    #[test]
    fn unc_and_verbatim_paths_are_stored_as_plain_paths() {
        let project = Path::new(r"D:\projects\p.arcline");
        assert_eq!(
            project_relative(project, Path::new(r"\\?\UNC\server\share\a b.mkv")),
            "//server/share/a b.mkv"
        );
        assert_eq!(
            project_relative(project, Path::new(r"\\?\C:\media\a.mkv")),
            "C:/media/a.mkv"
        );
    }
}
