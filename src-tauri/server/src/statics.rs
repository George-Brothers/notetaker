//! Serving the built UI from disk.
//!
//! Small, and the one genuinely dangerous file in this crate. A static file
//! server that resolves `../../../.ssh/id_rsa` hands over the user's whole home
//! directory, and this server can be bound to a network. So the path resolution
//! is a pure function, and it is tested against the attacks rather than the
//! happy path.

use std::path::{Component, Path, PathBuf};

/// Resolves a URL path to a file inside `root`, or `None` if it escapes.
///
/// The rule is deliberately strict: a request path may only contain plain names.
/// Any `..`, any absolute component, any Windows prefix like `C:` is refused
/// outright rather than normalized. Normalizing is where these bugs live —
/// `..%2f`, `....//`, and unicode variants all exist to defeat a clever
/// canonicalizer, and none of them survive "reject anything that is not a plain
/// name".
///
/// The cost is that a legitimate file called `..thing` is unreachable. No such
/// file is produced by a Vite build.
pub fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    // Strip a query string and fragment; the caller may or may not have.
    let path = url_path
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_start_matches('/');

    // A bare "/" (or "") is the app shell.
    let path = if path.is_empty() { "index.html" } else { path };

    // Percent-encoding is refused rather than decoded. The UI's own asset URLs
    // never contain it, and decoding is precisely how `..%2f` becomes `../`.
    if path.contains('%') || path.contains('\\') || path.contains('\0') {
        return None;
    }

    let candidate = Path::new(path);
    for component in candidate.components() {
        match component {
            Component::Normal(_) => {}
            // Anything else — ParentDir, RootDir, Prefix, CurDir — is a refusal.
            _ => return None,
        }
    }

    Some(root.join(candidate))
}

/// A guess at a Content-Type from the file extension.
///
/// Only the types a Vite build actually emits. An unknown extension gets
/// `application/octet-stream`, which a browser will download rather than
/// execute — the safe direction for a file we did not expect to be serving.
pub fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("map") => "application/json; charset=utf-8",
        // Recording audio, served by the `/audio/` route rather than from the
        // UI build. Without these a browser refuses to play the file at all —
        // `<audio>` will not touch `application/octet-stream`.
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/app/dist")
    }

    // --- the happy path -------------------------------------------------

    #[test]
    fn the_bare_root_serves_the_app_shell() {
        assert_eq!(
            resolve(&root(), "/"),
            Some(PathBuf::from("/app/dist/index.html"))
        );
        assert_eq!(
            resolve(&root(), ""),
            Some(PathBuf::from("/app/dist/index.html"))
        );
    }

    #[test]
    fn a_nested_asset_resolves_inside_the_root() {
        assert_eq!(
            resolve(&root(), "/assets/index-abc123.js"),
            Some(PathBuf::from("/app/dist/assets/index-abc123.js"))
        );
    }

    #[test]
    fn a_query_string_is_ignored() {
        assert_eq!(
            resolve(&root(), "/index.html?token=abc"),
            Some(PathBuf::from("/app/dist/index.html"))
        );
    }

    // --- the attacks ----------------------------------------------------

    /// The one that matters. This server can be bound to a network, so a
    /// traversal here is "hand over the home directory to anyone on the wifi".
    #[test]
    fn parent_directory_traversal_is_refused() {
        for attack in [
            "/../secrets.txt",
            "/../../etc/passwd",
            "/assets/../../../../etc/passwd",
            "..",
            "/..",
            "/a/../../b",
        ] {
            assert_eq!(
                resolve(&root(), attack),
                None,
                "traversal was allowed: {attack}"
            );
        }
    }

    /// Percent-encoded traversal. Refused by rejecting `%` outright rather than
    /// by decoding and re-checking, because decode-then-check is the pattern
    /// every bypass in this family targets.
    #[test]
    fn percent_encoded_traversal_is_refused() {
        for attack in [
            "/..%2fsecrets",
            "/%2e%2e/%2e%2e/etc/passwd",
            "/assets/%2e%2e%2findex.html",
            "/%00",
        ] {
            assert_eq!(
                resolve(&root(), attack),
                None,
                "encoded traversal was allowed: {attack}"
            );
        }
    }

    /// Backslashes are path separators on Windows, so `..\..\` traverses there
    /// even though it is a legal filename character on Unix.
    #[test]
    fn backslash_traversal_is_refused() {
        for attack in [
            r"/..\..\secrets",
            r"/assets\..\..\etc",
            r"\windows\system32",
        ] {
            assert_eq!(
                resolve(&root(), attack),
                None,
                "backslash traversal was allowed: {attack}"
            );
        }
    }

    /// A leading `//` is stripped along with the single slash, so `//etc/passwd`
    /// becomes the relative `etc/passwd` and lands *inside* the root. That is
    /// safe — it names a file the UI does not have, which 404s — and it is the
    /// behaviour worth pinning: the requirement is "cannot escape the root", not
    /// "must be rejected".
    #[test]
    fn a_doubled_leading_slash_stays_inside_the_root() {
        let resolved = resolve(&root(), "//etc/passwd").expect("should resolve, safely");
        assert_eq!(resolved, PathBuf::from("/app/dist/etc/passwd"));
        assert!(resolved.starts_with(root()));
    }

    /// A Windows drive prefix must not be reachable — `C:` as a component would
    /// make `root.join()` discard the root entirely.
    #[test]
    fn a_windows_drive_prefix_is_refused() {
        // On non-Windows these parse as Normal components rather than a Prefix,
        // so the colon form is caught by the join staying inside root; the
        // backslash form is refused outright. Both must be non-escaping.
        for attack in [r"C:\Windows\System32\config\SAM", "C:/Windows"] {
            let got = resolve(&root(), attack);
            if let Some(p) = got {
                assert!(
                    p.starts_with(root()),
                    "a drive prefix escaped the root: {p:?}"
                );
            }
        }
    }

    #[test]
    fn a_null_byte_is_refused() {
        assert_eq!(resolve(&root(), "/index.html\0.png"), None);
    }

    /// Every accepted path must land inside the root. The catch-all invariant,
    /// in case a future edit adds a component kind that slips past the match.
    #[test]
    fn anything_accepted_stays_inside_the_root() {
        let candidates = [
            "/",
            "/index.html",
            "/assets/a.js",
            "/a/b/c/d.png",
            "/..",
            "/../x",
            r"/..\x",
            "/%2e%2e",
            "//etc/passwd",
            "/a/../b",
        ];
        for c in candidates {
            if let Some(p) = resolve(&root(), c) {
                assert!(
                    p.starts_with(root()),
                    "{c} resolved outside the root: {p:?}"
                );
            }
        }
    }

    // --- content types --------------------------------------------------

    #[test]
    fn known_extensions_get_their_content_type() {
        assert_eq!(
            content_type(Path::new("a/index.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("a/index-abc.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("a.css")), "text/css; charset=utf-8");
        assert_eq!(content_type(Path::new("a.woff2")), "font/woff2");
        assert_eq!(content_type(Path::new("a.png")), "image/png");
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        assert_eq!(content_type(Path::new("A.PNG")), "image/png");
        assert_eq!(
            content_type(Path::new("INDEX.HTML")),
            "text/html; charset=utf-8"
        );
    }

    /// An unexpected file must be offered as a download rather than something
    /// the browser will run.
    #[test]
    fn an_unknown_extension_is_not_served_as_anything_executable() {
        for name in ["a.exe", "a", "a.weird", "a.php"] {
            assert_eq!(content_type(Path::new(name)), "application/octet-stream");
        }
    }
}
