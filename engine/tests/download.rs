//! End-to-end tests against a local server that throttles each connection,
//! the way many real servers do (and the reason multi-connection downloads win).

use rand::RngCore;
use rdm_engine::{client, start, Kind, Outcome, Request};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const MIB: usize = 1 << 20;

async fn serve(data: Arc<Vec<u8>>, ranges: bool, bytes_per_sec: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let data = data.clone();
            tokio::spawn(async move {
                loop {
                    let mut req = Vec::new();
                    let mut b = [0u8; 4096];
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut b).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => req.extend_from_slice(&b[..n]),
                        }
                    }
                    let req = String::from_utf8_lossy(&req).to_lowercase();
                    let last = data.len() - 1;
                    let range = req.lines().find_map(|l| l.strip_prefix("range: bytes=")).map(|r| {
                        let (a, b) = r.split_once('-').unwrap();
                        let b = b.trim().parse().unwrap_or(last).min(last);
                        (a.trim().parse::<usize>().unwrap(), b)
                    });
                    let (head, body) = match range.filter(|_| ranges) {
                        Some((a, b)) => (
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\n\
                                 Content-Disposition: attachment; filename=\"test.bin\"\r\n\r\n",
                                b - a + 1,
                                data.len()
                            ),
                            &data[a..=b],
                        ),
                        None => (format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", data.len()), &data[..]),
                    };
                    if sock.write_all(head.as_bytes()).await.is_err() {
                        return;
                    }
                    for chunk in body.chunks(64 * 1024) {
                        if sock.write_all(chunk).await.is_err() {
                            return;
                        }
                        tokio::time::sleep(Duration::from_secs_f64(chunk.len() as f64 / bytes_per_sec as f64)).await;
                    }
                }
            });
        }
    });
    format!("http://{addr}/files/test.bin")
}

fn random_data(len: usize) -> Arc<Vec<u8>> {
    let mut v = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut v);
    Arc::new(v)
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rdm-test-{}", rand::random::<u32>()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn request(url: &str, dir: &PathBuf, connections: usize) -> Request {
    Request { url: url.into(), headers: vec![], dir: dir.clone(), file_name: None, connections, kind: Kind::File, audio_url: None }
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_connection_is_correct_and_faster() {
    let data = random_data(16 * MIB);
    let url = serve(data.clone(), true, 2 * MIB).await;
    let dir = temp_dir();

    let t = Instant::now();
    let (_, task) = start(client(), request(&url, &dir, 8));
    let out = task.await.unwrap().unwrap();
    let elapsed = t.elapsed();

    assert_eq!(out, Outcome::Completed(dir.join("test.bin")));
    assert!(std::fs::read(dir.join("test.bin")).unwrap() == *data, "content mismatch");
    // One connection would need 8s at 2 MiB/s.
    assert!(elapsed < Duration::from_secs(4), "too slow: {elapsed:?}");
    assert!(!dir.join("test.bin.part.json").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn falls_back_without_range_support() {
    let data = random_data(3 * MIB);
    let url = serve(data.clone(), false, 64 * MIB).await;
    let dir = temp_dir();
    let (h, task) = start(client(), request(&url, &dir, 8));
    let out = task.await.unwrap().unwrap();
    assert_eq!(out, Outcome::Completed(dir.join("test.bin")));
    assert!(!h.progress().resumable);
    assert!(std::fs::read(dir.join("test.bin")).unwrap() == *data);
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_then_resume() {
    let data = random_data(16 * MIB);
    let url = serve(data.clone(), true, 2 * MIB).await;
    let dir = temp_dir();

    let (h, task) = start(client(), request(&url, &dir, 8));
    tokio::time::sleep(Duration::from_millis(800)).await;
    h.pause();
    assert_eq!(task.await.unwrap().unwrap(), Outcome::Paused);
    let partial = h.progress().received;
    assert!(partial > 0 && partial < data.len() as u64, "partial = {partial}");
    assert!(dir.join("test.bin.part.json").exists());

    let mut req = request(&url, &dir, 8);
    req.file_name = Some("test.bin".into());
    let (h, task) = start(client(), req);
    assert_eq!(task.await.unwrap().unwrap(), Outcome::Completed(dir.join("test.bin")));
    assert_eq!(h.progress().received, data.len() as u64);
    assert!(std::fs::read(dir.join("test.bin")).unwrap() == *data, "content mismatch after resume");
}
