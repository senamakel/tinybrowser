//! Minimal end-to-end usage of the crate.
//!
//! Examples are compiled and linted in CI, so they cannot drift from the API.
//! Run it with:
//!
//! ```sh
//! cargo run -p tinybrowser --example basic
//! ```
//!
//! It drives a real browser when this host has one and can reach the page.
//! When it cannot — a CI runner with no browser, a container with no egress —
//! it prints the same error a caller would get and exits successfully, because
//! both are facts about the machine rather than failures of the example.

use tinybrowser::{
    Action, Browser, Error, NavigateRequest, ReadRequest, SessionOptions, SnapshotRequest, Target,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let browser = Browser::new();

    let session = match browser.open_session(SessionOptions::default()).await {
        Ok(session) => session,
        Err(error @ Error::BrowserUnavailable { .. }) => {
            println!("no browser on this host, so there is nothing to drive: {error}");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    println!("opened {} at {}", session.id, session.endpoint);

    let page = match browser
        .navigate(&session.id, &NavigateRequest::new("https://example.com"))
        .await
    {
        Ok(page) => page,
        Err(error @ (Error::PageError { .. } | Error::Timeout { .. })) => {
            println!("cannot reach example.com from this host: {error}");
            browser.close_session(&session.id).await?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    println!("{} — {:?}", page.url, page.title);

    // What an agent reads: the accessibility tree, with a ref on everything it
    // could act on.
    let snapshot = browser
        .snapshot(&session.id, &SnapshotRequest::default())
        .await?;
    println!("\n{}\n", snapshot.tree);

    // And what it does with one of those refs.
    if let Some(link) = snapshot.refs.iter().find(|element| element.role == "link") {
        let outcome = browser
            .perform(
                &session.id,
                &Action::Click {
                    target: Target::reference(&link.id),
                    new_tab: false,
                },
            )
            .await?;
        println!("clicked {:?}, now at {}", link.name, outcome.page.url);
    }

    let text = browser
        .read_page(&session.id, &ReadRequest::default())
        .await?;
    println!("\n{}", text.content.chars().take(400).collect::<String>());

    browser.close_session(&session.id).await?;
    Ok(())
}
