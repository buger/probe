use chrono::Utc;
use std::process::Command;

fn main() {
    // Get git hash
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .map(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .unwrap_or_else(|_| "unknown".to_string())
                    .trim()
                    .to_string()
            } else {
                "unknown".to_string()
            }
        })
        .unwrap_or_else(|_| "unknown".to_string());

    // Get current UTC time
    let build_date = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    println!("cargo:rustc-env=GIT_HASH={git_hash}");
    println!("cargo:rustc-env=BUILD_DATE={build_date}");

    // Rerun if git changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads/");
}
