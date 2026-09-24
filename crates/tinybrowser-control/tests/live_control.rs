//! End-to-end Jev control against a real browser and local mock provider.

#![cfg(feature = "engine")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};
use tinybrowser::{Browser, NavigateRequest, SessionOptions};
use tinybrowser_control::{JevController, TaskRequest, TaskStatus};
use tinyjevclient::{Client, ClientConfig, RetryPolicy};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn live_controller_clicks_and_confirms_completion() {
    if std::env::var_os("TINYBROWSER_LIVE_TESTS").is_none() {
        return;
    }

    let (page_url, page_task) = serve_page().await;
    let (provider_url, provider_task) = serve_provider().await;

    let mut config = ClientConfig::new("test-key");
    config.base_url = provider_url;
    config.retry = RetryPolicy {
        max_retries: 0,
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(1),
    };
    let browser = Browser::new();
    let session = browser
        .open_session(SessionOptions::default())
        .await
        .expect("launch browser");
    browser
        .navigate(&session.id, &NavigateRequest::new(page_url))
        .await
        .expect("navigate to local page");

    let result = JevController::new(Client::new(config).expect("mock client"))
        .run(
            &browser,
            &session.id,
            &TaskRequest::new("Click Continue and reach Done"),
        )
        .await
        .expect("controller run");

    assert_eq!(result.status, TaskStatus::Done);
    assert_eq!(result.steps.len(), 1);
    assert!(result.final_snapshot.tree.contains("Done"));
    browser
        .close_session(&session.id)
        .await
        .expect("close browser");
    page_task.abort();
    provider_task.await.expect("provider completed");
}

#[tokio::test]
async fn live_openrouter_completes_a_multi_step_browser_task() {
    if std::env::var_os("TINYBROWSER_OPENROUTER_LIVE_TESTS").is_none() {
        return;
    }
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .expect("OPENROUTER_API_KEY must be set for the opted-in live test");
    let (page_url, page_task) = serve_form_page().await;
    let browser = Browser::new();
    let session = browser
        .open_session(SessionOptions::default())
        .await
        .expect("launch browser");
    browser
        .navigate(&session.id, &NavigateRequest::new(page_url))
        .await
        .expect("navigate to local form");
    let task = TaskRequest::new(
        "Reach the page state that visibly says Task complete by completing the form",
    )
    .with_inputs(BTreeMap::from([(
        "code word".to_owned(),
        "tinybrowser".to_owned(),
    )]));

    let result = JevController::new(
        Client::new(ClientConfig::openrouter(api_key)).expect("OpenRouter client"),
    )
    .run(&browser, &session.id, &task)
    .await
    .expect("live OpenRouter controller run");

    println!("OpenRouter calls: {}", result.steps.len() + 1);
    for record in &result.steps {
        println!(
            "step={} operation={:?} confidence={:.3} goal_done={:.3} latency_ms={} attempts={} changed={}",
            record.step,
            record.decision.operation,
            record.decision.confidence,
            record.decision.goal_done,
            record.decision.latency.as_millis(),
            record.decision.attempts,
            record.page_changed,
        );
    }
    if let Some(terminal) = &result.terminal {
        println!(
            "terminal={:?} confidence={:.3} goal_done={:.3} latency_ms={} attempts={} model={} input_tokens={:?} output_tokens={:?}",
            terminal.operation,
            terminal.confidence,
            terminal.goal_done,
            terminal.latency.as_millis(),
            terminal.attempts,
            terminal.model,
            terminal.usage.input_tokens,
            terminal.usage.output_tokens,
        );
    }
    println!("status={:?}", result.status);
    assert_eq!(result.status, TaskStatus::Done);
    assert!(result.final_snapshot.tree.contains("Task complete"));
    assert!(result.steps.len() >= 2, "expected fill and click steps");
    browser
        .close_session(&session.id)
        .await
        .expect("close browser");
    page_task.abort();
}

async fn serve_page() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind page server");
    let url = format!(
        "http://{}/",
        listener.local_addr().expect("page server address")
    );
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = vec![0_u8; 4_096];
                let _ = stream.read(&mut request).await;
                let body = r#"<!doctype html><title>Control test</title><button onclick="document.body.innerHTML='<p>Done</p>'">Continue</button>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (url, task)
}

async fn serve_form_page() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind form page server");
    let url = format!(
        "http://{}/",
        listener.local_addr().expect("form page server address")
    );
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = vec![0_u8; 4_096];
                let _ = stream.read(&mut request).await;
                let body = r#"<!doctype html><title>Live control</title>
                    <main><h1>Browser control check</h1>
                    <label>Code word <input aria-label="Code word"></label>
                    <button onclick="if(document.querySelector('input').value==='tinybrowser'){document.querySelector('main').innerHTML='<h1>Task complete</h1>'}else{document.querySelector('#status').textContent='Code word required'}">Continue</button>
                    <p id="status">Enter the requested code word, then continue.</p></main>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (url, task)
}

async fn serve_provider() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind provider server");
    let url = format!(
        "http://{}",
        listener.local_addr().expect("provider server address")
    );
    let task = tokio::spawn(async move {
        for selected in ["CLICK", "DONE"] {
            let (mut stream, _) = listener.accept().await.expect("provider request");
            let request = read_http_body(&mut stream).await;
            let questions = request["questions"].as_object().expect("question object");
            let operation = &questions["operation"]["criteria"];
            let probabilities = operation
                .as_object()
                .expect("operation criteria")
                .keys()
                .map(|key| (key.clone(), json!(f64::from(key == selected))))
                .collect::<BTreeMap<_, _>>();
            let mut answers = serde_json::Map::from_iter([(
                "operation".to_owned(),
                json!({
                    "type": "choice",
                    "choice": selected,
                    "probabilities": probabilities,
                    "confidence": 1.0,
                }),
            )]);
            answers.insert(
                "goal_done".to_owned(),
                json!({"type": "noul", "noul": if selected == "DONE" { 0.99 } else { 0.01 }}),
            );
            if let Some(targets) = questions.get("click_target") {
                let criteria = targets["criteria"].as_object().expect("target criteria");
                let target = criteria.keys().next().expect("click target");
                let target_probabilities = criteria
                    .keys()
                    .map(|key| (key.clone(), json!(f64::from(key == target))))
                    .collect::<BTreeMap<_, _>>();
                answers.insert(
                    "click_target".to_owned(),
                    json!({
                        "type": "choice",
                        "choice": target,
                        "probabilities": target_probabilities,
                        "confidence": 1.0,
                    }),
                );
            }
            let body = json!({
                "model": "jev-latest",
                "answers": answers,
                "usage": {"input_tokens": 10, "output_tokens": 2},
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("provider response");
        }
    });
    (url, task)
}

async fn read_http_body(stream: &mut TcpStream) -> Value {
    let mut received = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4_096];
        let read = stream
            .read(&mut chunk)
            .await
            .expect("read provider request");
        assert!(read > 0, "provider request ended before headers");
        received.extend_from_slice(&chunk[..read]);
        if let Some(index) = received.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&received[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("content length"))
        })
        .expect("content-length header");
    while received.len() < header_end + content_length {
        let mut chunk = [0_u8; 4_096];
        let read = stream.read(&mut chunk).await.expect("read provider body");
        assert!(read > 0, "provider request ended before body");
        received.extend_from_slice(&chunk[..read]);
    }
    serde_json::from_slice(&received[header_end..header_end + content_length])
        .expect("provider request JSON")
}
