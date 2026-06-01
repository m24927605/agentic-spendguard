use spendguard_tokenizer::Tokenizer;
use std::time::Instant;

fn main() {
    let started = Instant::now();
    let tokenizer = Tokenizer::new_with_embedded_assets().expect("boot tokenizer");
    let total_boot_ns = started.elapsed().as_nanos();

    println!("metric\tname\tvalue");
    println!("total_boot_ns\tall\t{total_boot_ns}");
    println!("dispatch_entries\tall\t{}", tokenizer.dispatch().len());
    for metric in tokenizer.encoder_boot_durations() {
        println!(
            "encoder_boot_ms\t{}\t{}",
            metric.encoder_name, metric.duration_ms
        );
    }
}
