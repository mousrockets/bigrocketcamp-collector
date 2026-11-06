//! Finds a newer release on GitHub and fetches its installer, checked against the release's SHA256SUMS.
//!
//! Nothing is installed on its own: the player clicks « Mettre à jour ».

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::Duration;
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub const INSTALLER: &str = "BigRocketCampCollector-setup.exe";
const SUMS: &str = "SHA256SUMS";
const MAX_INSTALLER: u64 = 64 * 1024 * 1024;
const MAX_ANSWER: u64 = 256 * 1024;

/// The repository of this build (Cargo.toml `repository`), so that a fork updates from its own releases.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    installer: String,
    sums: String,
}

/// `v1.2.3` or `1.2.3`; anything else (a pre-release, a typo) is ignored.
fn parse_version(text: &str) -> Option<[u64; 3]> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.').map(|part| part.parse::<u64>().ok());
    let version = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(version)
}

/// The latest release when it is newer than `current` and carries both the installer and its checksum.
fn newer_release(latest: &Value, current: &str) -> Option<Release> {
    let tag = latest["tag_name"].as_str()?;
    if parse_version(tag)? <= parse_version(current)? {
        return None;
    }
    let assets = latest["assets"].as_array()?;
    let url = |name: &str| {
        assets
            .iter()
            .find(|asset| asset["name"] == name)
            .and_then(|asset| asset["browser_download_url"].as_str())
            .filter(|url| url.starts_with("https://"))
            .map(str::to_owned)
    };
    Some(Release {
        version: tag.strip_prefix('v').unwrap_or(tag).to_owned(),
        installer: url(INSTALLER)?,
        sums: url(SUMS)?,
    })
}

/// The expected hash of `name` in a `sha256sum` listing: `<hex>  <name>` or `<hex> *<name>`.
fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_lowercase())
    })
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

pub struct Updater {
    agent: Agent,
    api: String,
}

impl Updater {
    pub fn new() -> Self {
        let tls = TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build();
        let agent = Agent::config_builder()
            .tls_config(tls)
            .https_only(true)
            // Release downloads redirect to GitHub's file storage.
            .max_redirects(5)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(300)))
            .user_agent(concat!("bigrocketcamp-collector/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent();
        let api = REPOSITORY.replacen("https://github.com/", "https://api.github.com/repos/", 1);
        Self { agent, api }
    }

    /// `Some(None)` when this version is the latest, `None` when GitHub could not be asked.
    pub fn check(&self) -> Option<Option<Release>> {
        let mut response = self
            .agent
            .get(format!("{}/releases/latest", self.api))
            .header("Accept", "application/vnd.github+json")
            .call()
            .ok()?;
        let text = response.body_mut().with_config().limit(MAX_ANSWER).read_to_string().ok()?;
        let latest: Value = serde_json::from_str(&text).ok()?;
        latest["tag_name"].as_str()?;
        Some(newer_release(&latest, current_version()))
    }

    /// Downloads the installer into the temporary folder, only if its SHA-256 matches the published one.
    pub fn download(&self, release: &Release) -> Result<PathBuf, &'static str> {
        let fetch = |url: &str, limit: u64| -> Option<Vec<u8>> {
            let mut response = self.agent.get(url).call().ok()?;
            response.body_mut().with_config().limit(limit).read_to_vec().ok()
        };
        let sums = fetch(&release.sums, MAX_ANSWER).ok_or("téléchargement impossible")?;
        let expected = expected_hash(&String::from_utf8_lossy(&sums), INSTALLER).ok_or("somme de contrôle absente")?;
        let installer = fetch(&release.installer, MAX_INSTALLER).ok_or("téléchargement impossible")?;
        if sha256(&installer) != expected {
            return Err("fichier altéré");
        }
        let path = std::env::temp_dir().join(format!("BigRocketCampCollector-setup-{}.exe", release.version));
        std::fs::write(&path, installer).map_err(|_| "écriture impossible")?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn latest(tag: &str) -> Value {
        json!({
            "tag_name": tag,
            "assets": [
                { "name": INSTALLER, "browser_download_url": "https://github.com/o/r/releases/download/v9.0.0/setup.exe" },
                { "name": "SHA256SUMS", "browser_download_url": "https://github.com/o/r/releases/download/v9.0.0/SHA256SUMS" }
            ]
        })
    }

    #[test]
    fn offers_only_a_newer_complete_release() {
        assert_eq!(
            newer_release(&latest("v0.10.0"), "0.9.3").map(|release| release.version).as_deref(),
            Some("0.10.0")
        );
        assert_eq!(newer_release(&latest("v0.2.0"), "0.2.0"), None);
        assert_eq!(newer_release(&latest("v0.1.0"), "0.2.0"), None);
        assert_eq!(newer_release(&latest("v1.0.0-beta.1"), "0.2.0"), None);
        let mut missing_sums = latest("v1.0.0");
        missing_sums["assets"].as_array_mut().unwrap().pop();
        assert_eq!(newer_release(&missing_sums, "0.2.0"), None);
    }

    #[test]
    fn reads_the_sha256sum_listing() {
        let hash = "a".repeat(64);
        let listing = format!("{hash}  {INSTALLER}\n{}  other.exe\n", "b".repeat(64));
        assert_eq!(expected_hash(&listing, INSTALLER), Some(hash.clone()));
        assert_eq!(expected_hash(&format!("{hash} *{INSTALLER}"), INSTALLER), Some(hash));
        assert_eq!(expected_hash("not-a-hash  BigRocketCampCollector-setup.exe", INSTALLER), None);
    }

    #[test]
    fn hashes_like_sha256sum() {
        assert_eq!(sha256(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn asks_the_api_of_this_repository() {
        assert_eq!(Updater::new().api, "https://api.github.com/repos/mousrockets/bigrocketcamp-collector");
    }
}
