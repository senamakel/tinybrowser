//! What a session is allowed to navigate to.
//!
//! # Why the check is here and not in the host
//!
//! A host can refuse to *ask* for a URL, and should. It cannot refuse the ones
//! it never sees: a click on a link, a redirect, a `window.location` in a script
//! on the page. Only the thing holding the browser is in a position to see every
//! destination, and this module is that thing.
//!
//! What is enforced here is the first half — the destinations that arrive as
//! requests. It is a guard rail against an agent wandering off, not a sandbox:
//! an allowlist that a page's own JavaScript can still navigate around is worth
//! having and worth being honest about. A host that needs a real boundary puts
//! the browser in a network namespace that only reaches what it should.

use url::Url;

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
/// (`https://example.com`, matched on scheme, host, and port) or a host with a
/// leading dot (`.example.com`, matching that host and every subdomain).
///
/// # Errors
///
/// [`Error::BlockedByPolicy`] when the allowlist is non-empty and nothing in it
/// matches.
pub(crate) fn check_allowed(url: &Url, allowed: &[String]) -> Result<()> {
    if allowed.is_empty() {
        return Ok(());
    }

    let Some(host) = url.host_str() else {
        return Err(Error::BlockedByPolicy {
            url: url.to_string(),
        });
    };

    let permitted = allowed.iter().any(|entry| {
        let entry = entry.trim();
        if let Some(suffix) = entry.strip_prefix('.') {
            // A leading dot means "this host and anything under it". Comparing
            // with the dot kept — `.example.com` against `evil-example.com` —
            // is what stops a suffix match from admitting a lookalike domain.
            return host.eq_ignore_ascii_case(suffix)
                || host
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{}", suffix.to_ascii_lowercase()));
        }

        match Url::parse(entry) {
            Ok(origin) => origin.origin() == url.origin(),
            // An entry that is neither an origin nor a dotted suffix is matched
            // as a bare host. Being lenient here is deliberate: an operator who
            // wrote `example.com` meant the site, and refusing to interpret it
            // would silently block everything instead.
            Err(_) => host.eq_ignore_ascii_case(entry),
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
