//! End-to-end HTTP integration tests for fasm-engine.
//!
//! Each test spins up a real engine on an OS-assigned port, fires HTTP
//! requests, and asserts on status codes and JSON bodies.
//!
//! All tests share a single Tokio runtime via `#[tokio::test]`.

mod common;
use common::TestEngine;

// ── Basic HTTP ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_ping_returns_200_json() {
    let engine = TestEngine::start_fixtures(128).await;
    let resp = engine.get("/ping").await;
    assert_eq!(resp.status().as_u16(), 200, "expected HTTP 200");
    let body: serde_json::Value = resp.json().await.expect("JSON body");
    assert_eq!(
        body,
        serde_json::json!(200),
        "Ping handler must return Int32(200)"
    );
}

#[tokio::test]
async fn test_echo_returns_path_param() {
    let engine = TestEngine::start_fixtures(128).await;
    let resp = engine.get("/echo/hello").await;
    assert_eq!(resp.status().as_u16(), 200);
    // The echo handler returns the bytes as a UTF-8 JSON string
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("hello"),
        "response body {:?} should contain 'hello'",
        body
    );
}

#[tokio::test]
async fn test_fib_returns_correct_result() {
    let engine = TestEngine::start_fixtures(128).await;
    let resp = engine.get("/fib").await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("JSON body");
    assert_eq!(body, serde_json::json!(832040), "fib(30) must equal 832040");
}

#[tokio::test]
async fn test_unknown_route_returns_404() {
    let engine = TestEngine::start_fixtures(128).await;
    let resp = engine.get("/no/such/path/exists").await;
    assert_eq!(resp.status().as_u16(), 404, "unregistered path must be 404");
}

// ── Admin endpoints ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_metrics_endpoint_returns_prometheus_text() {
    let engine = TestEngine::start_fixtures(128).await;
    // Warm up the invocation counter
    let _ = engine.get("/ping").await;
    let resp = engine.get("/metrics").await;
    assert_eq!(resp.status().as_u16(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("text/plain"),
        "content-type must be text/plain, got: {}",
        ct
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("fasm_invocations"),
        "metrics body must contain 'fasm_invocations'"
    );
}

#[tokio::test]
async fn test_admin_queues_returns_json() {
    let engine = TestEngine::start_fixtures(128).await;
    let resp = engine.get("/admin/queues").await;
    assert_eq!(resp.status().as_u16(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("application/json"),
        "expected JSON, got: {}",
        ct
    );
}

// ── Overload / back-pressure ──────────────────────────────────────────────────

/// With max_concurrent=1, firing many heavy requests concurrently means most will be
/// immediately bounced with 503.  We verify that exactly the back-pressure path works.
#[tokio::test]
async fn test_overload_returns_503() {
    // Only 1 concurrent execution allowed.
    let engine = TestEngine::start_fixtures(1).await;

    let base = engine.base_url.clone();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    // Fire 20 requests simultaneously — the fib handler is CPU-heavy.
    let futs: Vec<_> = (0..20)
        .map(|_| {
            let c = client.clone();
            let url = format!("{}/fib", base);
            tokio::spawn(async move { c.get(&url).send().await })
        })
        .collect();

    let mut got_200 = 0u32;
    let mut got_503 = 0u32;
    for f in futs {
        if let Ok(Ok(r)) = f.await {
            match r.status().as_u16() {
                200 => got_200 += 1,
                503 => got_503 += 1,
                other => eprintln!("unexpected status {}", other),
            }
        }
    }
    assert!(got_200 >= 1, "at least one request should succeed");
    assert!(got_503 >= 1, "with max_concurrent=1, most should 503");
    println!("[overload test] 200s={} 503s={}", got_200, got_503);
}

// ── Concurrent correctness ────────────────────────────────────────────────────

/// Fire 100 concurrent GET /ping — all must succeed (no panics, no data races).
#[tokio::test]
async fn test_100_concurrent_get_ping() {
    let engine = TestEngine::start_fixtures(128).await;
    let base = engine.base_url.clone();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    let futs: Vec<_> = (0..100)
        .map(|_| {
            let c = client.clone();
            let url = format!("{}/ping", base);
            tokio::spawn(async move { c.get(&url).send().await })
        })
        .collect();

    let mut failures = 0u32;
    for f in futs {
        match f.await {
            Ok(Ok(r)) if r.status().as_u16() == 200 => {}
            Ok(Ok(r)) => {
                eprintln!("unexpected status {}", r.status());
                failures += 1;
            }
            Err(e) => {
                eprintln!("join error: {}", e);
                failures += 1;
            }
            Ok(Err(e)) => {
                eprintln!("request error: {}", e);
                failures += 1;
            }
        }
    }
    assert_eq!(failures, 0, "{} requests failed out of 100", failures);
}

/// Fire 50 concurrent GET /echo/:word with unique words — all must round-trip
/// correctly (verifies no cross-contamination between executions).
#[tokio::test]
async fn test_concurrent_echo_no_cross_contamination() {
    let engine = TestEngine::start_fixtures(128).await;
    let base = engine.base_url.clone();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    let words: Vec<String> = (0..50).map(|i| format!("word{:04}", i)).collect();
    let futs: Vec<_> = words
        .iter()
        .map(|w| {
            let c = client.clone();
            let url = format!("{}/echo/{}", base, w);
            let expected = w.clone();
            tokio::spawn(async move {
                let r = c.get(&url).send().await?;
                let body = r.text().await?;
                Ok::<(String, String), reqwest::Error>((expected, body))
            })
        })
        .collect();

    for f in futs {
        let (expected, body) = f.await.unwrap().unwrap();
        assert!(
            body.contains(&expected),
            "echo mismatch: expected '{}' in body '{}' — possible cross-contamination!",
            expected,
            body
        );
    }
}
