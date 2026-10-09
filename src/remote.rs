//! HTTPS transport and Git smart-HTTP ref discovery shared by commands that read remote
//! repositories.
//!
//! Requests trust the operating system's certificate store and honor the proxy variables curl
//! reads, so networks with a private CA or an egress proxy work as they did with curl.

use std::env;
use std::fs;
use std::io::Read;

use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Proxy, ProxyProtocol};

use crate::test_hooks;

const GITHUB_BASE_URL: &str = "https://github.com";

pub(crate) fn remote_refs_url(repository: &str) -> String {
    format!("{GITHUB_BASE_URL}/{repository}.git/info/refs?service=git-upload-pack")
}

pub(crate) fn fetch_remote_tags(repository: &str) -> Result<Vec<String>, ()> {
    let advertisement = fetch_url(&remote_refs_url(repository))?;
    let tags = parse_remote_refs(&advertisement)
        .into_iter()
        .filter_map(|(_, reference)| reference.strip_prefix("refs/tags/").map(str::to_owned))
        // Git advertises an annotated tag twice: once as the tag object and once as `<tag>^{}`,
        // the commit it points at. Keep only the tag name itself.
        .filter(|tag| !tag.ends_with("^{}"))
        .collect::<Vec<_>>();
    let mut tags = tags;
    tags.sort();
    tags.dedup();
    (!tags.is_empty()).then_some(tags).ok_or(())
}

pub(crate) fn resolve_branch_commit(repository: &str, branch: &str) -> Result<String, ()> {
    let advertisement = fetch_url(&remote_refs_url(repository))?;
    let expected = format!("refs/heads/{branch}");
    parse_remote_refs(&advertisement)
        .into_iter()
        .find_map(|(sha, reference)| (reference == expected).then_some(sha))
        .ok_or(())
}

pub(crate) fn fetch_url(url: &str) -> Result<Vec<u8>, ()> {
    // Two test hooks exist because commands differ in how many requests they make. The manifest maps
    // several URLs to several responses, for commands that resolve a ref and then download an
    // archive. The single-response hook answers every request with the same bytes, optionally
    // asserting the URL, which is what a one-request command needs.
    if let Some(path) = test_hooks::var_os("OQTOPUS_TEST_HTTP_FIXTURE_MANIFEST") {
        let manifest = fs::read_to_string(path).map_err(|_| ())?;
        let fixture = manifest.lines().find_map(|line| {
            let (candidate, fixture) = line.split_once('\t')?;
            (candidate == url).then_some(fixture)
        });
        return fs::read(fixture.ok_or(())?).map_err(|_| ());
    }
    if let Some(path) = test_hooks::var_os("OQTOPUS_TEST_HTTP_RESPONSE_FILE") {
        if let Some(expected) = test_hooks::var("OQTOPUS_TEST_EXPECTED_HTTP_URL")
            && expected != url
        {
            return Err(());
        }
        return fs::read(path).map_err(|_| ());
    }
    let mut response = agent()?.get(url).call().map_err(|_| ())?;
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut body)
        .map_err(|_| ())?;
    Ok(body)
}

fn agent() -> Result<Agent, ()> {
    let proxy = https_proxy(|name| env::var(name).ok())?;
    Ok(Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .proxy(proxy)
        .build()
        .new_agent())
}

/// Selects the proxy for HTTPS requests from the variables curl reads, in curl's order.
///
/// Every URL this CLI fetches, including redirect targets, is HTTPS, so `http_proxy` never
/// applies. Empty variables count as unset, and an unparsable proxy fails the request rather
/// than silently connecting directly.
fn https_proxy(var: impl Fn(&str) -> Option<String>) -> Result<Option<Proxy>, ()> {
    let set = |name: &str| var(name).filter(|value| !value.is_empty());
    let Some(spec) = ["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"]
        .into_iter()
        .find_map(set)
    else {
        return Ok(None);
    };
    let no_proxy = set("no_proxy")
        .or_else(|| set("NO_PROXY"))
        .unwrap_or_default();

    let with_scheme = if spec.contains("://") {
        spec
    } else {
        format!("http://{spec}")
    };
    let uri = with_scheme.parse::<Uri>().map_err(|_| ())?;
    let protocol = ProxyProtocol::try_from(uri.scheme_str().unwrap_or("http")).map_err(|_| ())?;
    let authority = uri.authority().ok_or(())?;
    let mut builder = Proxy::builder(protocol).host(authority.host());
    if let Some(port) = authority.port_u16() {
        builder = builder.port(port);
    }
    if let Some((userinfo, _)) = authority.as_str().rsplit_once('@') {
        builder = match userinfo.split_once(':') {
            Some((username, password)) => builder.username(username).password(password),
            None => builder.username(userinfo),
        };
    }
    // curl trims each entry and treats `example.com` and `.example.com` alike, matching the
    // domain and every subdomain. ureq matches a bare name exactly and a dotted one as a suffix
    // only, so each entry is registered in both forms.
    for entry in no_proxy
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        if entry == "*" {
            builder = builder.no_proxy(entry);
            continue;
        }
        let domain = entry.trim_start_matches('.');
        builder = builder.no_proxy(domain).no_proxy(&format!(".{domain}"));
    }
    builder.build().map(Some).map_err(|_| ())
}

/// Extracts `(sha, reference)` pairs from a git smart-HTTP advertisement.
///
/// The advertisement is scanned for `<40 hex digits> refs/...` rather than decoded as pkt-lines.
/// Only the ref names matter here, and scanning tolerates the framing variations that appear
/// around the first ref: a pkt-line length prefix, the `service=git-upload-pack` header, and the
/// NUL-separated capability list. A 40-digit hexadecimal prefix is specific enough that no other
/// part of the advertisement matches it.
fn parse_remote_refs(advertisement: &[u8]) -> Vec<(String, String)> {
    let mut refs = Vec::new();
    let marker = b" refs/";

    for marker_start in find_all(advertisement, marker) {
        // A SHA-1 object name is 40 hexadecimal digits, and it ends where the marker's space
        // begins.
        if marker_start < 40
            || !advertisement[marker_start - 40..marker_start]
                .iter()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            continue;
        }
        // Skip the marker's leading space; the reference itself starts at `refs/`.
        let reference_start = marker_start + 1;
        // A ref name ends at the next separator: a newline between entries, or the NUL that
        // introduces the capability list on the first one.
        let reference_end = advertisement[reference_start..]
            .iter()
            .position(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
            .map_or(advertisement.len(), |offset| reference_start + offset);
        let Ok(reference) = std::str::from_utf8(&advertisement[reference_start..reference_end])
        else {
            continue;
        };
        let Ok(sha) = std::str::from_utf8(&advertisement[marker_start - 40..marker_start]) else {
            continue;
        };
        refs.push((sha.to_owned(), reference.to_owned()));
    }

    refs
}

fn find_all<'a>(haystack: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(move |(index, window)| (window == needle).then_some(index))
}

#[cfg(test)]
mod tests {
    use super::{https_proxy, parse_remote_refs};
    use ureq::ProxyProtocol;
    use ureq::http::Uri;

    fn proxy_for(vars: &[(&str, &str)]) -> Result<Option<ureq::Proxy>, ()> {
        https_proxy(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    fn bypasses(proxy: &ureq::Proxy, host: &str) -> bool {
        proxy.is_no_proxy(&format!("https://{host}/").parse::<Uri>().unwrap())
    }

    #[test]
    fn proxy_selection_follows_curl_precedence_for_https() {
        assert!(proxy_for(&[]).unwrap().is_none());
        // curl never uses http_proxy for an HTTPS URL.
        assert!(
            proxy_for(&[("http_proxy", "h:1"), ("HTTP_PROXY", "h:1")])
                .unwrap()
                .is_none()
        );
        assert!(proxy_for(&[("https_proxy", "")]).unwrap().is_none());

        let host = |vars: &[(&str, &str)]| proxy_for(vars).unwrap().unwrap().host().to_owned();
        assert_eq!(
            host(&[("https_proxy", "lower"), ("HTTPS_PROXY", "upper")]),
            "lower"
        );
        assert_eq!(
            host(&[("HTTPS_PROXY", "specific"), ("all_proxy", "all")]),
            "specific"
        );
        assert_eq!(host(&[("https_proxy", ""), ("ALL_PROXY", "all")]), "all");
    }

    #[test]
    fn proxy_specification_matches_curl_defaults() {
        let proxy = proxy_for(&[("https_proxy", "user:p%40ss@proxy.example:3128")])
            .unwrap()
            .unwrap();
        assert_eq!(proxy.protocol(), ProxyProtocol::Http);
        assert_eq!(proxy.host(), "proxy.example");
        assert_eq!(proxy.port(), 3128);
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p%40ss"));

        let socks = proxy_for(&[("ALL_PROXY", "socks5h://gateway:1080")])
            .unwrap()
            .unwrap();
        assert_eq!(socks.protocol(), ProxyProtocol::Socks5h);
        assert_eq!(socks.port(), 1080);

        assert!(proxy_for(&[("https_proxy", "ftp://proxy.example")]).is_err());
    }

    #[test]
    fn no_proxy_matches_domains_like_curl() {
        let proxy = proxy_for(&[
            ("https_proxy", "proxy.example:3128"),
            ("NO_PROXY", " github.com , .internal,localhost"),
        ])
        .unwrap()
        .unwrap();
        assert!(bypasses(&proxy, "github.com"));
        assert!(bypasses(&proxy, "codeload.github.com"));
        assert!(bypasses(&proxy, "internal"));
        assert!(bypasses(&proxy, "a.b.internal"));
        assert!(bypasses(&proxy, "localhost"));
        assert!(!bypasses(&proxy, "notgithub.com"));
        assert!(!bypasses(&proxy, "objects.githubusercontent.com"));

        let lowercase_wins = proxy_for(&[
            ("https_proxy", "proxy.example"),
            ("no_proxy", "a.example"),
            ("NO_PROXY", "*"),
        ])
        .unwrap()
        .unwrap();
        assert!(!bypasses(&lowercase_wins, "github.com"));

        let all = proxy_for(&[("https_proxy", "proxy.example"), ("no_proxy", "*")])
            .unwrap()
            .unwrap();
        assert!(bypasses(&all, "github.com"));
    }

    #[test]
    fn parses_unpeeled_tag_refs() {
        let refs = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa refs/tags/v1.2.3\0capability\n\
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb refs/tags/v2.0.0^{}\n\
cccccccccccccccccccccccccccccccccccccccc refs/tags/v2.0.0-rc.1\n";

        assert_eq!(
            parse_remote_refs(refs),
            [
                (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                    "refs/tags/v1.2.3".to_owned()
                ),
                (
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
                    "refs/tags/v2.0.0^{}".to_owned()
                ),
                (
                    "cccccccccccccccccccccccccccccccccccccccc".to_owned(),
                    "refs/tags/v2.0.0-rc.1".to_owned()
                ),
            ]
        );
    }
}
