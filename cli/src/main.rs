use clap::Parser;
use rdm_engine::{client, start, Kind, Outcome, Request};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Command-line front end, mainly for benchmarking the engine.
#[derive(Parser)]
struct Args {
    url: String,
    /// Target directory
    #[arg(short, long, default_value = ".")]
    dir: PathBuf,
    /// Parallel connections
    #[arg(short, long, default_value_t = 8)]
    connections: usize,
    /// Extra header, e.g. -H "Cookie: a=b" (repeatable)
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,
    /// Save under this name (also resumes a paused download of that name)
    #[arg(short, long)]
    output: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let headers = args
        .headers
        .iter()
        .filter_map(|h| h.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let kind = if args.url.split('?').next().unwrap_or("").ends_with(".m3u8") { Kind::Hls } else { Kind::File };
    let req = Request { url: args.url, headers, dir: args.dir, file_name: args.output, connections: args.connections, kind };

    let t0 = Instant::now();
    let (handle, mut task) = start(client(), req);
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let (mut last, mut last_t, mut peak_conns) = (0u64, Instant::now(), 0);
    let result = loop {
        tokio::select! {
            r = &mut task => break r?,
            _ = tick.tick() => {
                let p = handle.progress();
                let now = Instant::now();
                let speed = (p.received - last) as f64 / now.duration_since(last_t).as_secs_f64();
                (last, last_t) = (p.received, now);
                peak_conns = peak_conns.max(p.connections);
                let pct = p.total.map(|t| format!("{:5.1}%", p.received as f64 * 100.0 / t as f64))
                    .or(p.pieces.map(|[d, t]| format!("{d}/{t} pieces")))
                    .unwrap_or_default();
                print!("\r{pct} {:>10} {:>10}/s  conns {:>2}  segs {:>3}   ",
                    mib(p.received), mib(speed as u64), p.connections, p.segments.len());
                std::io::stdout().flush().ok();
            }
        }
    };
    let p = handle.progress();
    let secs = t0.elapsed().as_secs_f64();
    println!();
    match result? {
        Outcome::Completed(path) => println!(
            "done: {} in {secs:.2}s, avg {}/s, peak conns {peak_conns}, segments {}",
            path.display(),
            mib((p.received as f64 / secs) as u64),
            p.segments.len()
        ),
        Outcome::Paused => println!("paused"),
    }
    Ok(())
}

fn mib(b: u64) -> String {
    format!("{:.2} MiB", b as f64 / (1 << 20) as f64)
}
