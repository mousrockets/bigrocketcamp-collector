//! The three calls the collector makes to Big Rocket Camp (see « API attendue » in the README).

use crate::bridge::Reply;
use serde_json::{Value, json};
use std::time::Duration;
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

/// The Big Rocket Camp instance this build talks to; another instance rebuilds with its own APP_URL.
/// An empty APP_URL (an unset CI variable) counts as no APP_URL.
pub const APP_URL: &str = match option_env!("APP_URL") {
    Some(url) if !url.is_empty() => url,
    _ => "https://mousrockets.com",
};

const MAX_ANSWER: u64 = 64 * 1024;

pub struct Api {
    agent: Agent,
    base: String,
}

pub enum Pairing {
    Paired { token: String, player: String },
    Refused(String),
    Offline,
}

impl Api {
    pub fn new() -> Self {
        let tls = TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build();
        let agent = Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("bigrocketcamp-collector/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent();
        Self { agent, base: APP_URL.trim_end_matches('/').to_owned() }
    }

    /// `POST /api/live` with the scoreboard exactly as the browser bridge sends it.
    pub fn send_match(&self, token: &str, body: &str) -> Reply {
        let request = self
            .agent
            .post(format!("{}/api/live", self.base))
            .header("Authorization", format!("Bearer {token}"))
            .content_type("application/json");
        answer(request.send(body))
    }

    /// Trades the code shown in the member's profile for a token.
    pub fn pair(&self, code: &str) -> Pairing {
        let body = json!({ "code": code }).to_string();
        let request = self.agent.post(format!("{}/api/collector/pair", self.base)).content_type("application/json");
        let Some((status, text)) = answer(request.send(body)) else {
            return Pairing::Offline;
        };
        let reply: Value = serde_json::from_str(&text).unwrap_or_default();
        match (status, reply["token"].as_str(), reply["player"].as_str()) {
            (200, Some(token), Some(player)) if !token.is_empty() => {
                Pairing::Paired { token: token.to_owned(), player: player.to_owned() }
            }
            (404, ..) => Pairing::Refused("Le site ne propose pas encore l’association.".into()),
            _ => Pairing::Refused(reply["error"].as_str().unwrap_or("Code invalide ou expiré.").to_owned()),
        }
    }

    /// The paired player, `Some(None)` once the token is revoked, and `None` when the API cannot be reached.
    pub fn player(&self, token: &str) -> Option<Option<String>> {
        let request = self
            .agent
            .get(format!("{}/api/collector/status", self.base))
            .header("Authorization", format!("Bearer {token}"));
        let (status, text) = answer(request.call())?;
        match status {
            401 => Some(None),
            200 => {
                let reply: Value = serde_json::from_str(&text).ok()?;
                Some(Some(reply["player"].as_str()?.to_owned()))
            }
            _ => None,
        }
    }
}

fn answer(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Reply {
    let mut response = result.ok()?;
    let status = response.status().as_u16();
    let text = response.body_mut().with_config().limit(MAX_ANSWER).read_to_string().unwrap_or_default();
    Some((status, text))
}
