//! Opt-in live `OpenRouter` task against Trip.com.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use tinybrowser::{Browser, NavigateRequest, SessionOptions};
use tinybrowser_control::{ControlLimits, JevController, TaskRequest, TaskStatus};
use tinyjevclient::{Client, ClientConfig};

#[tokio::test]
async fn live_trip_com_finds_mumbai_to_goa_flights() {
    if std::env::var_os("TINYBROWSER_TRIP_LIVE_TESTS").is_none() {
        return;
    }
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .expect("OPENROUTER_API_KEY must be set for the opted-in Trip.com test");
    let browser = Browser::new();
    let session = browser
        .open_session(SessionOptions {
            headless: false,
            ..SessionOptions::default()
        })
        .await
        .expect("launch browser");
    browser
        .navigate(
            &session.id,
            &NavigateRequest::new("https://www.trip.com/flights/city-bom-airport-goi/"),
        )
        .await
        .expect("open Trip.com Mumbai to Goa route");
    let task = TaskRequest::new(
        "Find visible one-way flight options from Mumbai (BOM) to Goa Dabolim (GOI). Stop when route flight listings with airlines, times, or fares are visible. Do not book, purchase, sign in, or enter passenger details.",
    );
    let controller = JevController::new(
        Client::new(ClientConfig::openrouter(api_key)).expect("OpenRouter client"),
    )
    .with_limits(ControlLimits {
        max_steps: 24,
        max_unchanged_steps: 5,
        wait_ms: 750,
        ..ControlLimits::default()
    });

    let result = controller
        .run(&browser, &session.id, &task)
        .await
        .expect("Trip.com controller run");

    println!(
        "Trip.com status={:?} calls={} url={} title={}",
        result.status,
        result.steps.len() + usize::from(result.terminal.is_some()),
        result.final_snapshot.url,
        result.final_snapshot.title,
    );
    for record in &result.steps {
        println!(
            "step={} operation={:?} target={:?} input={:?} confidence={:.3} goal_done={:.3} latency_ms={} changed={} outcome={:?}",
            record.step,
            record.decision.operation,
            record.decision.target.as_ref().map(|target| &target.name),
            record.decision.input_name,
            record.decision.confidence,
            record.decision.goal_done,
            record.decision.latency.as_millis(),
            record.page_changed,
            record.outcome,
        );
    }
    if let Some(terminal) = &result.terminal {
        println!(
            "terminal={:?} confidence={:.3} goal_done={:.3} latency_ms={} model={} input_tokens={:?} output_tokens={:?} request_id={:?}",
            terminal.operation,
            terminal.confidence,
            terminal.goal_done,
            terminal.latency.as_millis(),
            terminal.model,
            terminal.usage.input_tokens,
            terminal.usage.output_tokens,
            terminal.request_id,
        );
    }
    let snapshot_excerpt = result
        .final_snapshot
        .tree
        .chars()
        .take(4_000)
        .collect::<String>();
    println!("final snapshot excerpt:\n{snapshot_excerpt}");

    assert!(matches!(
        result.status,
        TaskStatus::Done | TaskStatus::DoneUnconfirmed
    ));
    let evidence = format!(
        "{}\n{}\n{}",
        result.final_snapshot.url, result.final_snapshot.title, result.final_snapshot.tree
    )
    .to_lowercase();
    assert!(evidence.contains("mumbai") || evidence.contains("bom"));
    assert!(evidence.contains("goa") || evidence.contains("goi"));
    browser
        .close_session(&session.id)
        .await
        .expect("close browser");
}
