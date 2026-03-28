use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::process::{Command, Output};

pub fn command_output_checked(cmd: &mut Command, context: &str) -> Result<Output> {
    let output = cmd.output().with_context(|| context.to_string())?;
    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "command exited with a non-zero status"
    };
    anyhow::bail!("{context}: {detail}");
}

pub fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let left = a.as_bytes();
    let right = b.as_bytes();
    let mut diff = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for idx in 0..max_len {
        let left_byte = *left.get(idx).unwrap_or(&0);
        let right_byte = *right.get(idx).unwrap_or(&0);
        diff |= (left_byte ^ right_byte) as usize;
    }
    diff == 0
}

pub fn normalize_sha256_hex(value: &str) -> Result<String> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.len() != 64 || !trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        anyhow::bail!("token_sha256 must be a 64-character hex string");
    }
    Ok(trimmed)
}
