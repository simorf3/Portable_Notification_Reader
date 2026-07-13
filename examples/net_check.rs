use portable_notification_reader::{edge_tts, voices};

#[tokio::main]
async fn main() {
    println!("== Fetching full online voice catalog ==");
    let list = voices::fetch_online_voices();
    println!("Fetched {} online voices", list.len());
    if list.is_empty() {
        eprintln!("WARN: live fetch failed; would fall back to {} seeded voices", voices::fallback_online_voices().len());
    } else {
        let za: Vec<_> = list.iter().filter(|v| v.locale.starts_with("en-ZA") || v.locale.ends_with("ZA")).collect();
        println!("South African voices found: {}", za.len());
        for v in &za { println!("   - {} [{}] {}", v.token, v.locale, v.display); }
        // show how many english voices
        let en = list.iter().filter(|v| v.language() == "en").count();
        println!("English voices total: {}", en);
    }

    println!("\n== Synthesizing a phrase with en-ZA-LeahNeural ==");
    match edge_tts::synthesize("en-ZA-LeahNeural", "Hello from South Africa. This is a portable notification reader test.", 0).await {
        Ok(mp3) => {
            std::fs::write("/tmp/za_test.mp3", &mp3).unwrap();
            println!("OK: received {} bytes of MP3 -> /tmp/za_test.mp3", mp3.len());
        }
        Err(e) => eprintln!("SYNTH FAILED: {e:#}"),
    }
}
