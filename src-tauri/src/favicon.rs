//! Finding and caching the icon a remote project serves.
//!
//! Asking the webview to load `https://host/favicon.ico` directly does not work in
//! practice: most sites declare their icon in a `<link rel="icon">` and never serve that
//! path, and a host that simply never answers leaves a request hanging with nothing to fall
//! back to. Both problems want the same answer — look the icon up once, here, and keep the
//! bytes.
//!
//! So this module reads the page, finds what it declares, downloads it, and writes it into
//! Oracle's icon directory. From then on it is an ordinary local image: it renders instantly,
//! costs no network, and survives the project being offline.

use crate::config;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;

/// Long enough for a slow VPS, short enough that a dead host does not hold a task open.
const TIMEOUT: Duration = Duration::from_secs(8);

/// Plenty for any icon, and a bound on what a hostile response can make Oracle buffer.
const MAX_BYTES: usize = 512 * 1024;

/// Only this much of the page is read: `<link>` tags live in `<head>`.
const MAX_HTML: usize = 96 * 1024;

/// A client of its own, because this one must follow redirects.
///
/// The health-check client deliberately does not — a deployment that has started
/// redirecting is breakage worth reporting. Here a redirect is just how sites move you from
/// `http` to `https`, or from `/` to a locale, and refusing to follow would find no icon at
/// all.
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("Oracle/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// The cached icon for this URL, if one has already been fetched.
pub fn cached(page_url: &str) -> Option<PathBuf> {
    let stem = cache_stem(page_url);
    let dir = config::icons_dir();

    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.file_stem().is_some_and(|found| found == stem.as_str()))
}

/// Resolves the icon for a page and writes it to the cache, returning where it landed.
///
/// Returns `None` for anything that does not lead to a usable image, which is a normal
/// outcome: plenty of projects are bare APIs with no icon to find.
pub async fn resolve(page_url: &str) -> Option<PathBuf> {
    if let Some(hit) = cached(page_url) {
        return Some(hit);
    }

    let client = client();
    let html = fetch_text(&client, page_url).await.unwrap_or_default();

    for candidate in candidates(&html, page_url) {
        if let Some(path) = try_download(&client, &candidate, page_url).await {
            return Some(path);
        }
    }

    None
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Option<String> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }

    let bytes = response.bytes().await.ok()?;
    let end = bytes.len().min(MAX_HTML);

    // Lossy on purpose: a page in an unexpected encoding should still yield its ASCII tags
    // rather than nothing at all.
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

async fn try_download(
    client: &reqwest::Client,
    url: &str,
    page_url: &str,
) -> Option<PathBuf> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }

    let extension = extension_for(&response, url)?;
    let bytes = response.bytes().await.ok()?;

    // A zero-length body, or an HTML error page served with a 200, is not an icon.
    if bytes.is_empty() || bytes.len() > MAX_BYTES || looks_like_markup(&bytes) {
        return None;
    }

    let dir = config::icons_dir();
    std::fs::create_dir_all(&dir).ok()?;

    let path = dir.join(format!("{}.{extension}", cache_stem(page_url)));
    std::fs::write(&path, &bytes).ok()?;

    Some(path)
}

/// The extension to store under, from the content type first and the URL second.
///
/// The extension is what the asset protocol uses to decide a MIME type when the webview
/// loads the file back, so a wrong one means an icon that downloads and then will not
/// render.
fn extension_for(response: &reqwest::Response, url: &str) -> Option<&'static str> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let from_type = match content_type.split(';').next().unwrap_or("").trim() {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/svg+xml" => Some("svg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/x-icon" | "image/vnd.microsoft.icon" => Some("ico"),
        _ => None,
    };

    if from_type.is_some() {
        return from_type;
    }

    // Servers are careless with content types on icons; the path usually tells the truth.
    let path = url.split(['?', '#']).next().unwrap_or("");
    match path.rsplit('.').next()?.to_ascii_lowercase().as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "svg" => Some("svg"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        "ico" => Some("ico"),
        _ => None,
    }
}

/// True when the body is HTML or XML rather than an image.
///
/// The common failure is a site answering every unknown path with its 200 index page. An
/// SVG is XML too, so it is recognised and allowed through.
fn looks_like_markup(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(256)];
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    let trimmed = text.trim_start();

    if trimmed.contains("<svg") {
        return false;
    }

    trimmed.starts_with("<!doctype html") || trimmed.starts_with("<html")
}

/// Icon URLs to try, best first, always ending with `/favicon.ico`.
fn candidates(html: &str, page_url: &str) -> Vec<String> {
    let mut declared = declared_icons(html);

    // Anything the page declares comes first, then the path every browser falls back to.
    if let Some(fallback) = fallback_url(page_url) {
        declared.push(fallback);
    }

    declared
        .into_iter()
        .filter_map(|href| absolute(&href, page_url))
        .collect()
}

/// The `href`s of every `<link>` whose `rel` mentions an icon, in document order.
///
/// A deliberately small scanner rather than an HTML parser: the question is narrow, the
/// input is one tag type, and a dependency that can parse all of HTML would be a large
/// amount of machinery for `rel` and `href`.
fn declared_icons(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut found = Vec::new();
    let mut touch = Vec::new();
    let mut cursor = 0;

    while let Some(start) = lower[cursor..].find("<link").map(|at| cursor + at) {
        let end = match lower[start..].find('>') {
            Some(at) => start + at,
            None => break,
        };
        cursor = end + 1;

        let tag = &lower[start..end];
        let Some(rel) = attribute(tag, "rel") else {
            continue;
        };
        if !rel.contains("icon") {
            continue;
        }

        // Slice the original, not the lowercased copy: paths are case-sensitive.
        let Some(href) = attribute(&html[start..end], "href") else {
            continue;
        };
        if href.is_empty() {
            continue;
        }

        // A touch icon is a last resort: it is meant for a home screen, so it is large and
        // often has its corners pre-rounded for a platform that is not this one.
        if rel.contains("apple-touch") {
            touch.push(href);
        } else {
            found.push(href);
        }
    }

    found.extend(touch);
    found
}

/// The value of an attribute inside a single tag, quoted or bare.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut cursor = 0;

    while let Some(at) = lower[cursor..].find(name).map(|at| cursor + at) {
        cursor = at + name.len();

        // Must be a whole attribute name: `rel` must not match inside `hreflang`.
        let before_ok = tag[..at]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || c == '<');
        let rest = tag[cursor..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }

        let value = rest[1..].trim_start();
        let quote = value.chars().next()?;

        return if quote == '"' || quote == '\'' {
            value[1..].split(quote).next().map(|v| v.trim().to_string())
        } else {
            value
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()
                .map(|v| v.trim().to_string())
        };
    }

    None
}

/// `scheme://host[:port]/favicon.ico` for an HTTP URL, or `None` for anything else.
fn fallback_url(url: &str) -> Option<String> {
    Some(format!("{}/favicon.ico", origin(url)?))
}

/// The `scheme://authority` part of an HTTP URL.
fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }

    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() {
        return None;
    }

    Some(format!("{scheme}://{authority}"))
}

/// Turns an `href` from a page into an absolute URL.
fn absolute(href: &str, page_url: &str) -> Option<String> {
    let href = href.trim();

    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    // A data URI is already the image; nothing to fetch, and nothing sane to cache.
    if href.starts_with("data:") {
        return None;
    }

    let origin = origin(page_url)?;

    if let Some(rest) = href.strip_prefix("//") {
        let scheme = origin.split("://").next()?;
        return Some(format!("{scheme}://{rest}"));
    }
    if href.starts_with('/') {
        return Some(format!("{origin}{href}"));
    }

    // Relative to the page's directory. Good enough for the depth an icon link ever has.
    let path = page_url.split(['?', '#']).next().unwrap_or(page_url);
    let base = match path.rfind('/') {
        Some(at) if at > origin.len() => &path[..at],
        _ => origin.as_str(),
    };

    Some(format!("{base}/{href}"))
}

/// A stable, filesystem-safe name for a page's cached icon.
fn cache_stem(page_url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    page_url.hash(&mut hasher);
    format!("fav-{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://veln.dev/dashboard?tab=1";

    #[test]
    fn the_fallback_is_the_origin_plus_the_conventional_path() {
        assert_eq!(
            fallback_url(PAGE).as_deref(),
            Some("https://veln.dev/favicon.ico")
        );
        // The realistic self-hosted case: plain http, an address, a port.
        assert_eq!(
            fallback_url("http://89.107.53.161:9400").as_deref(),
            Some("http://89.107.53.161:9400/favicon.ico")
        );
    }

    #[test]
    fn only_http_urls_have_an_origin() {
        for url in ["ssh://box/thing", "file:///c:/tmp", "not a url", "https://"] {
            assert_eq!(origin(url), None, "{url}");
        }
    }

    #[test]
    fn a_declared_icon_is_preferred_over_the_fallback() {
        let html = r#"<html><head><link rel="icon" href="/brand/mark.png"></head>"#;

        assert_eq!(
            candidates(html, PAGE),
            vec![
                "https://veln.dev/brand/mark.png".to_string(),
                "https://veln.dev/favicon.ico".to_string(),
            ]
        );
    }

    #[test]
    fn a_touch_icon_is_tried_after_a_real_one() {
        let html = r#"
            <link rel="apple-touch-icon" sizes="180x180" href="/touch.png">
            <link rel="shortcut icon" href="/small.ico">
        "#;

        let found = declared_icons(html);
        assert_eq!(found, vec!["/small.ico".to_string(), "/touch.png".to_string()]);
    }

    #[test]
    fn links_that_are_not_icons_are_ignored() {
        let html = r#"
            <link rel="stylesheet" href="/app.css">
            <link rel="preconnect" href="https://fonts.example">
        "#;

        assert!(declared_icons(html).is_empty());
    }

    #[test]
    fn attributes_are_read_in_every_spelling_pages_use() {
        assert_eq!(
            declared_icons(r#"<link href='/a.png' rel='icon'>"#),
            vec!["/a.png".to_string()]
        );
        assert_eq!(
            declared_icons("<link rel=icon href=/b.png>"),
            vec!["/b.png".to_string()]
        );
        // Upper-case tags and attributes are still HTML.
        assert_eq!(
            declared_icons(r#"<LINK REL="ICON" HREF="/C.png">"#),
            vec!["/C.png".to_string()]
        );
    }

    #[test]
    fn an_href_is_never_matched_inside_another_attribute() {
        // `hreflang` contains `href`, and reading it would yield a language tag.
        let html = r#"<link rel="icon" hreflang="en" href="/real.png">"#;
        assert_eq!(declared_icons(html), vec!["/real.png".to_string()]);
    }

    #[test]
    fn every_form_of_href_resolves() {
        assert_eq!(
            absolute("https://cdn.example/i.png", PAGE).as_deref(),
            Some("https://cdn.example/i.png")
        );
        assert_eq!(
            absolute("//cdn.example/i.png", PAGE).as_deref(),
            Some("https://cdn.example/i.png")
        );
        assert_eq!(
            absolute("/i.png", PAGE).as_deref(),
            Some("https://veln.dev/i.png")
        );
        assert_eq!(
            absolute("i.png", PAGE).as_deref(),
            Some("https://veln.dev/i.png")
        );
        assert_eq!(
            absolute("i.png", "https://veln.dev/a/b/page").as_deref(),
            Some("https://veln.dev/a/b/i.png")
        );
        // An inline icon is already the image, so there is nothing to fetch.
        assert_eq!(absolute("data:image/png;base64,AAAA", PAGE), None);
    }

    #[test]
    fn an_index_page_served_for_a_missing_icon_is_rejected() {
        assert!(looks_like_markup(b"<!DOCTYPE html><html><body>404"));
        assert!(looks_like_markup(b"<html lang=\"en\">"));
        // An SVG is markup and a perfectly good icon.
        assert!(!looks_like_markup(br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#));
        assert!(!looks_like_markup(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn the_cache_name_is_stable_and_per_url() {
        assert_eq!(cache_stem(PAGE), cache_stem(PAGE));
        assert_ne!(cache_stem(PAGE), cache_stem("https://other.example"));
        assert!(cache_stem(PAGE).starts_with("fav-"));
    }
}
