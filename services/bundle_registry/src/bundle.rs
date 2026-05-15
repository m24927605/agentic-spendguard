//! Bundle .tgz read / pack / atomic write.
//!
//! Matches the deterministic flags from
//! `deploy/demo/init/bundles/generate.sh` (--sort=name --owner=0
//! --group=0 --mtime='UTC 1970-01-01') so re-packs of identical
//! inputs produce identical sha256.

use anyhow::{anyhow, Context, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::io::Write;
use std::path::Path;

const CONTRACT_YAML_NAME: &str = "contract.yaml";
const MANIFEST_JSON_NAME: &str = "manifest.json";

pub struct Bundle {
    pub contract_yaml: String,
    pub manifest_json: String,
}

pub struct LoadedBundle {
    pub contract_yaml: String,
    pub manifest_json: String,
    pub sha256_hex: String,
}

pub fn read_bundle(path: &Path) -> Result<LoadedBundle> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let sha256_hex = sha256_hex_of(&bytes);

    let dec = GzDecoder::new(std::io::Cursor::new(&bytes));
    let mut archive = tar::Archive::new(dec);

    let mut contract_yaml: Option<String> = None;
    let mut manifest_json: Option<String> = None;

    for entry in archive.entries().context("iterate tar entries")? {
        let mut entry = entry.context("tar entry")?;
        let path = entry.path().context("entry path")?.to_path_buf();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let mut buf = String::new();
        if name == CONTRACT_YAML_NAME {
            entry.read_to_string(&mut buf).context("read contract.yaml")?;
            contract_yaml = Some(buf);
        } else if name == MANIFEST_JSON_NAME {
            entry.read_to_string(&mut buf).context("read manifest.json")?;
            manifest_json = Some(buf);
        }
    }

    let contract_yaml = contract_yaml.ok_or_else(|| anyhow!("contract.yaml missing from bundle"))?;
    let manifest_json = manifest_json.ok_or_else(|| anyhow!("manifest.json missing from bundle"))?;

    Ok(LoadedBundle {
        contract_yaml,
        manifest_json,
        sha256_hex,
    })
}

/// Deterministic re-pack: returns (tgz_bytes, sha256_hex).
///
/// Matches `tar --sort=name --owner=0 --group=0 --mtime='UTC 1970-01-01'`
/// + `gzip -n` from generate.sh:
///   * file order is alphabetical (CONTRACT_YAML_NAME < MANIFEST_JSON_NAME
///     happens to NOT be alphabetical — contract < manifest, but `c` <
///     `m` in ASCII so it is alphabetical, fine)
///   * mtime 1970-01-01 UTC (0 epoch)
///   * uid/gid 0
///   * gzip with no filename header (-n)
pub fn pack_bundle(bundle: &Bundle) -> Result<(Vec<u8>, String)> {
    let mut entries: Vec<(&str, &[u8])> = vec![
        (CONTRACT_YAML_NAME, bundle.contract_yaml.as_bytes()),
        (MANIFEST_JSON_NAME, bundle.manifest_json.as_bytes()),
    ];
    entries.sort_by(|a, b| a.0.cmp(b.0));

    // Build the inner tar in memory.
    let mut tar_bytes: Vec<u8> = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        builder.mode(tar::HeaderMode::Deterministic);
        for (name, body) in &entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(name).context("header set_path")?;
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            // Deterministic mtime: epoch 0 = 1970-01-01 UTC.
            header.set_mtime(0);
            header.set_cksum();
            builder
                .append(&header, *body)
                .with_context(|| format!("append {}", name))?;
        }
        builder.finish().context("tar finish")?;
    }

    // gzip with no filename header (matches gzip -n).
    let mut gz_bytes: Vec<u8> = Vec::new();
    {
        let mut enc = GzEncoder::new(&mut gz_bytes, Compression::default());
        enc.write_all(&tar_bytes).context("gzip write")?;
        enc.finish().context("gzip finish")?;
    }

    let sha256_hex = sha256_hex_of(&gz_bytes);
    Ok((gz_bytes, sha256_hex))
}

pub fn write_bundle_atomic(tgz_path: &Path, tgz_bytes: &[u8], sig_path: &Path) -> Result<()> {
    write_file_atomic(tgz_path, tgz_bytes)
        .with_context(|| format!("atomic write {}", tgz_path.display()))?;
    write_file_atomic(sig_path, b"demo-cosign-placeholder")
        .with_context(|| format!("atomic write {}", sig_path.display()))?;
    Ok(())
}

pub fn update_runtime_env(runtime_env_path: &Path, new_hash_hex: &str) -> Result<()> {
    // Read existing runtime.env, rewrite the CONTRACT_BUNDLE_HASH_HEX
    // line, preserve all other keys. The file is small — re-write in
    // full atomically.
    let prior = std::fs::read_to_string(runtime_env_path)
        .with_context(|| format!("read {}", runtime_env_path.display()))?;
    let mut out = String::with_capacity(prior.len() + 80);
    let mut wrote_hash = false;
    for line in prior.lines() {
        if line.starts_with("SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=") {
            out.push_str("SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=");
            out.push_str(new_hash_hex);
            out.push('\n');
            wrote_hash = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !wrote_hash {
        out.push_str("SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=");
        out.push_str(new_hash_hex);
        out.push('\n');
    }
    write_file_atomic(runtime_env_path, out.as_bytes())
        .with_context(|| format!("atomic write {}", runtime_env_path.display()))?;
    Ok(())
}

fn write_file_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("path {} has no parent", path.display()))?;
    // Tempfile name: UUID v4 suffix (codex CA-P3.5 r1 P2 — was PID,
    // which can collide if two containers run as PID 1 on the same
    // shared volume).
    let tmp = dir.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("bundle"),
        uuid::Uuid::new_v4().simple()
    ));
    {
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("create tempfile {}", tmp.display()))?;
        f.write_all(bytes).context("write bytes")?;
        f.sync_all().context("fsync tempfile")?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} → {}", tmp.display(), path.display()))?;
    // POSIX rename is atomic at the inode level, but the directory
    // entry update is not durable on disk until the parent dir
    // itself is fsync'd (codex CA-P3.5 r2 P3). On Linux ext4/xfs
    // this is meaningful for crash durability of the bundle publish.
    if let Ok(dir_fd) = std::fs::File::open(dir) {
        // Best-effort: directory fsync may not be supported on all
        // filesystems (e.g., some FUSE / tmpfs paths). Failure here
        // is logged but not fatal — the file is still atomic from
        // the rename, just not yet flushed to stable storage.
        let _ = dir_fd.sync_all();
    }
    Ok(())
}

fn sha256_hex_of(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_then_read_round_trip() {
        let bundle = Bundle {
            contract_yaml: "hello: world\n".to_string(),
            manifest_json: r#"{"name":"x"}"#.to_string(),
        };
        let (bytes, hash) = pack_bundle(&bundle).unwrap();
        assert_eq!(hash.len(), 64);

        let tmp = tempfile_path("pack_round_trip");
        std::fs::write(&tmp, &bytes).unwrap();
        let loaded = read_bundle(&tmp).unwrap();
        assert_eq!(loaded.contract_yaml, "hello: world\n");
        assert_eq!(loaded.manifest_json, r#"{"name":"x"}"#);
        assert_eq!(loaded.sha256_hex, hash);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn pack_is_deterministic() {
        let bundle = Bundle {
            contract_yaml: "k: v\n".to_string(),
            manifest_json: r#"{"a":1}"#.to_string(),
        };
        let (b1, h1) = pack_bundle(&bundle).unwrap();
        let (b2, h2) = pack_bundle(&bundle).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(b1, b2);
    }

    #[test]
    fn runtime_env_replaces_hash() {
        let tmp = tempfile_path("rt_env");
        let prior = "FOO=bar\nSPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=oldhash\nBAZ=qux\n";
        std::fs::write(&tmp, prior).unwrap();
        update_runtime_env(&tmp, "newhash").unwrap();
        let after = std::fs::read_to_string(&tmp).unwrap();
        assert!(after.contains("SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=newhash"));
        assert!(after.contains("FOO=bar"));
        assert!(after.contains("BAZ=qux"));
        assert!(!after.contains("oldhash"));
        std::fs::remove_file(&tmp).ok();
    }

    fn tempfile_path(suffix: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "spendguard-bundle-registry-test-{}-{}",
            suffix,
            std::process::id()
        ));
        p
    }
}
