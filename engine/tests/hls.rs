//! HLS end to end: master playlist -> best variant -> mixed plain/AES-128 segments -> one file.

use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use rand::RngCore;
use rdm_engine::{client, start, Kind, Outcome, Request};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve(files: HashMap<String, Vec<u8>>) -> String {
    let files = Arc::new(files);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let files = files.clone();
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
                    let path = String::from_utf8_lossy(&req).split(' ').nth(1).unwrap_or("").to_string();
                    let resp = match files.get(&path) {
                        Some(body) => [format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes(), body.clone()].concat(),
                        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
                    };
                    if sock.write_all(&resp).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_best_variant_with_encrypted_segments() {
    let key = [7u8; 16];
    let mut files = HashMap::new();
    let mut expected = Vec::new();
    let mut playlist = String::from("#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:3\n");
    for i in 0..5u64 {
        let mut seg = vec![0u8; 50_000 + i as usize];
        rand::thread_rng().fill_bytes(&mut seg);
        expected.extend_from_slice(&seg);
        let body = if i >= 2 {
            if i == 2 {
                playlist.push_str("#EXT-X-KEY:METHOD=AES-128,URI=\"/key.bin\"\n");
            }
            let iv = ((3 + i) as u128).to_be_bytes(); // no IV attribute -> media sequence number
            cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(&seg)
        } else {
            seg
        };
        playlist.push_str(&format!("#EXTINF:4.0,\ns{i}.ts\n"));
        files.insert(format!("/hi/s{i}.ts"), body);
    }
    playlist.push_str("#EXT-X-ENDLIST\n");
    files.insert("/hi/index.m3u8".into(), playlist.into_bytes());
    files.insert("/key.bin".into(), key.to_vec());
    files.insert(
        "/master.m3u8".into(),
        b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=300000\nlo/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=4000000\nhi/index.m3u8\n".to_vec(),
    );
    let base = serve(files).await;

    let dir = std::env::temp_dir().join(format!("rdm-hls-{}", rand::random::<u32>()));
    std::fs::create_dir_all(&dir).unwrap();
    let req = Request {
        url: format!("{base}/master.m3u8"),
        headers: vec![],
        dir: dir.clone(),
        file_name: Some("out.ts".into()),
        connections: 4,
        kind: Kind::Hls,
        audio_url: None,
    };
    let (h, task) = start(client(), req);
    let Outcome::Completed(path) = task.await.unwrap().unwrap() else { panic!("not completed") };
    assert_eq!(path, dir.join("out.ts"));
    assert!(std::fs::read(&path).unwrap() == expected, "content mismatch");
    assert_eq!(h.progress().pieces, Some([5, 5]));
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_video_and_separate_audio_file_together() {
    let mut video = vec![0u8; 300_000];
    let mut audio = vec![0u8; 40_000];
    rand::thread_rng().fill_bytes(&mut video);
    rand::thread_rng().fill_bytes(&mut audio);
    let mut files = HashMap::new();
    files.insert("/v.mp4".to_string(), video.clone());
    files.insert("/a.m4a".to_string(), audio.clone());
    let base = serve(files).await;

    let dir = std::env::temp_dir().join(format!("rdm-av-{}", rand::random::<u32>()));
    std::fs::create_dir_all(&dir).unwrap();
    let req = Request {
        url: format!("{base}/v.mp4"),
        headers: vec![],
        dir: dir.clone(),
        file_name: Some("clip.mp4".into()),
        connections: 8,
        kind: Kind::File,
        audio_url: Some(format!("{base}/a.m4a")),
    };
    let (h, task) = start(client(), req);
    let Outcome::Completed(path) = task.await.unwrap().unwrap() else { panic!("not completed") };
    let p = h.progress();
    assert_eq!(p.received, (video.len() + audio.len()) as u64);
    // Random bytes can't be muxed, so both tracks are kept and the video is reported.
    assert_eq!(path, dir.join("clip.video"));
    assert!(std::fs::read(&path).unwrap() == video, "video mismatch");
    assert!(std::fs::read(dir.join("clip.audio")).unwrap() == audio, "audio mismatch");
    assert!(!dir.join("clip.mp4").exists());
}

fn random_segments(prefix: &str, n: usize, files: &mut HashMap<String, Vec<u8>>) -> (String, Vec<u8>) {
    let mut playlist = String::from("#EXTM3U\n");
    let mut all = Vec::new();
    for i in 0..n {
        let mut seg = vec![0u8; 30_000];
        rand::thread_rng().fill_bytes(&mut seg);
        all.extend_from_slice(&seg);
        playlist.push_str(&format!("#EXTINF:4.0,\n{i}.ts\n"));
        files.insert(format!("/{prefix}/{i}.ts"), seg);
    }
    playlist.push_str("#EXT-X-ENDLIST\n");
    files.insert(format!("/{prefix}/index.m3u8"), playlist.into_bytes());
    (format!("/{prefix}/index.m3u8"), all)
}

#[tokio::test(flavor = "multi_thread")]
async fn offers_qualities_and_fetches_separate_audio() {
    let mut files = HashMap::new();
    let (_, _) = random_segments("v1080", 3, &mut files);
    let (v360, video) = random_segments("v360", 3, &mut files);
    let (_, audio) = random_segments("aud", 2, &mut files);
    files.insert(
        "/master.m3u8".into(),
        b"#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"en\",DEFAULT=YES,URI=\"aud/index.m3u8\"\n\
          #EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=640x360,AUDIO=\"a\"\nv360/index.m3u8\n\
          #EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,AUDIO=\"a\"\nv1080/index.m3u8\n"
            .to_vec(),
    );
    let base = serve(files).await;

    let variants = rdm_engine::hls_variants(&client(), &format!("{base}/master.m3u8"), &[]).await.unwrap();
    let labels: Vec<_> = variants.iter().map(|v| v.label.as_str()).collect();
    assert_eq!(labels, ["1080p · 6.0 Mbps", "360p · 900 kbps"]);
    let chosen = &variants[1];
    assert_eq!(chosen.url, format!("{base}{v360}"));

    let dir = std::env::temp_dir().join(format!("rdm-hls-{}", rand::random::<u32>()));
    std::fs::create_dir_all(&dir).unwrap();
    let req = Request {
        url: chosen.url.clone(),
        headers: vec![],
        dir: dir.clone(),
        file_name: Some("clip.ts".into()),
        connections: 4,
        kind: Kind::Hls,
        audio_url: chosen.audio_url.clone(),
    };
    let (h, task) = start(client(), req);
    let Outcome::Completed(path) = task.await.unwrap().unwrap() else { panic!("not completed") };
    assert_eq!(h.progress().pieces, Some([5, 5]));
    // Random bytes are not real media, so ffmpeg can't mux them: both tracks must be kept side by side.
    assert_eq!(path, dir.join("clip.video.ts"));
    assert!(std::fs::read(&path).unwrap() == video, "video mismatch");
    assert!(std::fs::read(dir.join("clip.audio.ts")).unwrap() == audio, "audio mismatch");
    assert!(!dir.join("clip.mp4").exists());
}
