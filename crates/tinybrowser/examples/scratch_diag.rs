//! Scratch diagnostic, deleted before commit.
use tinybrowser::{Action, Browser, EvaluateRequest, NavigateRequest, SessionOptions, Target};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let b = Browser::new();
    let s = b.open_session(SessionOptions::default()).await?;
    b.navigate(&s.id, &NavigateRequest::new("https://example.com"))
        .await?;

    let outcome = b
        .perform(
            &s.id,
            &Action::Click {
                target: Target::selector("a"),
                new_tab: false,
            },
        )
        .await?;
    println!("after click: {}", outcome.page.url);

    for i in 0..10 {
        let v = b
            .evaluate(
                &s.id,
                &EvaluateRequest::new(
                    "({ ready: document.readyState, href: location.href, body: !!document.body, frames: frames.length })",
                ),
            )
            .await;
        println!("  t+{}: {:?}", i, v);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    b.close_session(&s.id).await?;
    Ok(())
}
