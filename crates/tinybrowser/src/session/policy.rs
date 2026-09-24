//! What a session is allowed to navigate to.
//!
//! # Why the check is here and not in the host
//!
//! A host can refuse to *ask* for a URL, and should. It cannot refuse the ones
//! it never sees: a click on a link, a redirect, a `window.location` in a script
//! on the page. Only the thing holding the browser is in a position to see every
//! destination, and this module is that thing.
//!
//! Explicit requests are checked here. Sessions with an allowlist also enable
//! [`super::navigation_guard`] so Chrome pauses document requests before network
//! egress, including clicks and redirects. This is still a navigation policy,
//! not a network sandbox: scripts and subresources can contact other hosts.
//! A host needing a whole-network boundary isolates the browser process.

use std::net::Ipv4Addr;

use url::{Host, Url};

use crate::error::{Error, Result};

/// Normalises what a caller asked for into a URL a browser can be sent to.
///
/// A bare host — `example.com`, `localhost:3000` — becomes `https://`, because
/// that is what an operator typing it means and what every address bar does.
/// Anything else must carry its own scheme, and only `http` and `https` are
/// accepted: `file:` would make the browser a filesystem reader for whoever can
/// reach the bus, and `javascript:` would make navigation an evaluation channel
/// that skips [`crate::Browser::evaluate`] and its deadline.
///
/// # Errors
///
/// [`Error::InvalidInput`] when the input is empty, unparseable, or carries a
/// scheme this module will not navigate to.
pub(crate) fn normalize_url(raw: &str) -> Result<Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::invalid_input("navigation url is empty"));
    }

    // `about:blank` is the one non-http destination worth admitting: it is how a
    // caller clears the page without closing the session.
    if trimmed.eq_ignore_ascii_case("about:blank") {
        return Url::parse("about:blank")
            .map_err(|error| Error::invalid_input(format!("about:blank: {error}")));
    }

    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    let url = Url::parse(&candidate)
        .map_err(|error| Error::invalid_input(format!("{trimmed} is not a url: {error}")))?;

    match url.scheme() {
        "http" | "https" => Ok(url),
        scheme => Err(Error::invalid_input(format!(
            "{scheme}: urls are not navigable; use http or https"
        ))),
    }
}

/// Whether `url` is admitted by `allowed`.
///
/// An empty allowlist admits everything: a session that did not ask for a
/// boundary does not get one imposed on it. An entry is either an origin
/// (`https://example.com`, matched on scheme, host, and port), a host with a
/// leading dot (`.example.com`, matching that host and every subdomain), or a
/// scheme-qualified host tree (`https://.example.com`, matching HTTPS only).
///
/// # Errors
///
/// [`Error::BlockedByPolicy`] when the allowlist is non-empty and nothing in it
/// matches.
pub(crate) fn check_allowed(url: &Url, allowed: &[String]) -> Result<()> {
    if allowed.is_empty() {
        return Ok(());
    }

    // `about:blank` is not a destination on the network and cannot carry
    // anything back; it is how a caller clears the page. Refusing it would mean
    // a session that sets an allowlist can never let go of the last page it
    // loaded, which is the opposite of what the setting is for.
    if url.scheme() == "about" {
        return Ok(());
    }

    let Some(host) = url.host_str() else {
        return Err(Error::BlockedByPolicy {
            url: url.to_string(),
        });
    };

    let permitted = allowed.iter().any(|entry| {
        let entry = entry.trim();
        if entry == "https://.*" {
            return url.scheme() == "https" && public_host_literal(url);
        }
        if let Some((scheme, suffix)) = entry.split_once("://.") {
            return matches!(scheme, "http" | "https")
                && url.scheme() == scheme
                && !suffix.is_empty()
                && !suffix.contains('/')
                && !suffix.contains(':')
                && host_matches_suffix(host, suffix);
        }
        if let Some(suffix) = entry.strip_prefix('.') {
            // A leading dot means "this host and anything under it". Comparing
            // with the dot kept — `.example.com` against `evil-example.com` —
            // is what stops a suffix match from admitting a lookalike domain.
            return host_matches_suffix(host, suffix);
        }

        // An entry with a scheme is an origin, and matched as one.
        if entry.contains("://") {
            return Url::parse(entry).is_ok_and(|origin| origin.origin() == url.origin());
        }

        // Everything else is a host, optionally with a port. Parsing it as a URL
        // would be wrong in a way that fails closed and looks like a typo:
        // `localhost:3000` parses happily as the scheme `localhost` with the
        // path `3000`, matches no origin at all, and silently blocks every
        // destination the operator meant to allow.
        match entry.rsplit_once(':') {
            Some((entry_host, port)) if port.chars().all(|c| c.is_ascii_digit()) => {
                host.eq_ignore_ascii_case(entry_host)
                    && url.port_or_known_default().map(|actual| actual.to_string())
                        == Some(port.to_string())
            }
            // Being lenient about a bare host is deliberate: an operator who
            // wrote `example.com` meant the site, and refusing to interpret it
            // would silently block everything instead.
            _ => host.eq_ignore_ascii_case(entry),
        }
    });

    if permitted {
        Ok(())
    } else {
        Err(Error::BlockedByPolicy {
            url: url.to_string(),
        })
    }
}

fn host_matches_suffix(host: &str, suffix: &str) -> bool {
    host.eq_ignore_ascii_case(suffix)
        || host
            .to_ascii_lowercase()
            .ends_with(&format!(".{}", suffix.to_ascii_lowercase()))
}

/// Public HTTPS host floor for the explicit allow-all navigation pattern.
/// DNS can still resolve a public name to a private address; hosts requiring
/// connection-level isolation must enforce that outside this URL guard.
fn public_host_literal(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => {
            let normalized = host.trim_end_matches('.').to_ascii_lowercase();
            !(normalized == "localhost"
                || normalized.ends_with(".localhost")
                || normalized == "local"
                || normalized
                    .rsplit_once('.')
                    .is_some_and(|(_, tld)| tld == "local"))
        }
        Some(Host::Ipv4(ip)) => public_ipv4(ip),
        Some(Host::Ipv6(ip)) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return public_ipv4(mapped);
            }
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast())
        }
        None => false,
    }
}

fn public_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || octets[0] == 0
        || octets[0] >= 240
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 198 && (18..=19).contains(&octets[1])))
}
