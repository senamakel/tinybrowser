//! Scratch diagnostic, deleted before commit.
use tinybrowser::{Action, Browser, EvaluateRequest, NavigateRequest, SessionOptions, Target};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let b = Browser::new();
    let s = b.open_session(SessionOptions::default()).await?;
    let page = "data:text/html,<form id=f action='https://example.com/' method='get'><input id='q' name='q'><button type='submit'>Go</button></form>";
    // data: is refused by policy, so use an inline document instead.
    let _ = page;
    b.navigate(&s.id, &NavigateRequest::new("https://example.com")).await?;
    b.evaluate(&s.id, &EvaluateRequest::new(
        "document.body.innerHTML = \"<form id=f action='/submitted' method='get'><input id='q' name='q'><button type='submit'>Go</button></form>\"; \
         document.addEventListener('submit', () => { window.__submitted = true; }, true); true",
    )).await?;

    b.perform(&s.id, &Action::Focus { target: Target::selector("#q") }).await?;
    let active = b.evaluate(&s.id, &EvaluateRequest::new("document.activeElement && document.activeElement.id")).await?;
    println!("activeElement id = {active:?}");

    b.perform(&s.id, &Action::Press { key: "Enter".to_string() }).await?;
    for i in 0..8 {
        let v = b
            .evaluate(&s.id, &EvaluateRequest::new("[!!window.__submitted, location.href, document.readyState]"))
            .await;
        println!("  t+{i}: {v:?}");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    b.close_session(&s.id).await?;
    Ok(())
}
