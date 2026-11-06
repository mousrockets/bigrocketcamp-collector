//! Keeps captured scoreboards on disk until the API confirms them: the queue and upload of `live-bridge.ts`.

use crate::capture::{check_live_match, receipt_id};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

pub const MAX_QUEUE: usize = 20;

/// The answer of `POST /api/live`, or `None` when the request did not reach the API.
pub type Reply = Option<(u16, String)>;

/// What happened to one queued match, for the status shown to the player.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Received,
    Dropped(String),
    Offline,
    Unconfirmed,
    Failed(String),
    Revoked,
}

/// What the uploader does next.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// Nothing left to send: wait for the next match.
    Idle,
    /// Try again in a little while.
    Retry,
    /// The collector must be paired again before anything is sent.
    Pair,
}

pub struct Queue {
    items: Vec<Value>,
    path: PathBuf,
}

impl Queue {
    /// The saved queue, without matches the API would now refuse (more than a day old, or corrupted).
    pub fn load(path: PathBuf, now: i64) -> Self {
        let saved: Vec<Value> = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        let skip = saved.len().saturating_sub(MAX_QUEUE);
        let items = saved.into_iter().skip(skip).filter(|payload| check_live_match(payload, now).is_ok()).collect();
        Self { items, path }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Keeps a new scoreboard, replacing an older one of the same match; returns false when the oldest was dropped.
    pub fn add(&mut self, payload: Value) -> bool {
        let guid = &payload["state"]["MatchGuid"];
        match self.items.iter().position(|queued| &queued["state"]["MatchGuid"] == guid) {
            Some(index) => self.items[index] = payload,
            None => self.items.push(payload),
        }
        let kept = self.items.len() <= MAX_QUEUE;
        if !kept {
            self.items.remove(0);
        }
        self.persist();
        kept
    }

    fn shift(&mut self) {
        self.items.remove(0);
        self.persist();
    }

    /// Written only when a match arrives or leaves: never while a match is being played.
    fn persist(&self) {
        if self.items.is_empty() {
            let _ = fs::remove_file(&self.path);
            return;
        }
        let draft = self.path.with_extension("tmp");
        let body = Value::Array(self.items.clone()).to_string();
        if fs::write(&draft, body).is_ok() {
            let _ = fs::rename(&draft, &self.path);
        }
    }

    /// Sends queued matches, oldest first, until one cannot be sent now.
    pub fn upload(&mut self, now: i64, mut post: impl FnMut(&str) -> Reply, mut report: impl FnMut(Event)) -> Next {
        while let Some(payload) = self.items.first() {
            if check_live_match(payload, now).is_err() {
                self.shift();
                report(Event::Dropped("Un match trop ancien n’a pas pu être envoyé.".into()));
                continue;
            }
            let expected = receipt_id(payload);
            let Some((status, body)) = post(&payload.to_string()) else {
                report(Event::Offline);
                return Next::Retry;
            };
            let reply: Value = serde_json::from_str(&body).unwrap_or_default();
            if !(200..300).contains(&status) {
                if status == 401 {
                    report(Event::Revoked);
                    return Next::Pair;
                }
                let error = reply["error"].as_str().unwrap_or("Envoi refusé par le site.").to_owned();
                // A refused scoreboard will never be accepted: drop it instead of retrying forever.
                // 403: none of its players belongs to the paired member.
                if matches!(status, 400 | 403 | 413) {
                    self.shift();
                    report(Event::Dropped(error));
                    continue;
                }
                report(Event::Failed(error));
                return Next::Retry;
            }
            if reply["ok"] != true || reply["match"].as_str() != Some(expected.as_str()) {
                report(Event::Unconfirmed);
                return Next::Retry;
            }
            self.shift();
            report(Event::Received);
        }
        Next::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::tests_support::{NOW, payload};

    fn queue(name: &str) -> Queue {
        let path = std::env::temp_dir().join(format!("brc-collector-{name}-{}.json", std::process::id()));
        let _ = fs::remove_file(&path);
        Queue::load(path, NOW)
    }

    fn receipt(guid: &str) -> Reply {
        Some((200, format!(r#"{{"ok":true,"match":"{guid}","received":6}}"#)))
    }

    #[test]
    fn keeps_a_match_across_a_lost_answer_then_sends_it_once() {
        let mut first = queue("resume");
        first.add(payload("resumed-match"));
        let mut events = Vec::new();
        assert_eq!(first.upload(NOW, |_| None, |event| events.push(event)), Next::Retry);
        assert_eq!(events, [Event::Offline]);

        let mut restarted = Queue::load(first.path.clone(), NOW);
        assert_eq!(restarted.len(), 1);
        let mut bodies = Vec::new();
        let next = restarted.upload(
            NOW,
            |body| {
                bodies.push(body.to_owned());
                receipt("resumed-match")
            },
            |_| {},
        );
        assert_eq!(next, Next::Idle);
        assert_eq!(bodies, [payload("resumed-match").to_string()]);
        assert_eq!(restarted.len(), 0);
        assert!(!restarted.path.exists());
    }

    #[test]
    fn drops_refused_matches_and_keeps_the_others_for_later() {
        let mut q = queue("refused");
        q.add(payload("refused-match"));
        q.add(payload("foreign-match"));
        q.add(payload("later-match"));
        let mut events = Vec::new();
        let next = q.upload(
            NOW,
            |body| {
                if body.contains("refused-match") {
                    Some((400, r#"{"error":"Tableau final incomplet."}"#.into()))
                } else if body.contains("foreign-match") {
                    Some((403, r#"{"error":"Aucun joueur du membre."}"#.into()))
                } else {
                    Some((503, String::new()))
                }
            },
            |event| events.push(event),
        );
        assert_eq!(next, Next::Retry);
        assert_eq!(
            events,
            [
                Event::Dropped("Tableau final incomplet.".into()),
                Event::Dropped("Aucun joueur du membre.".into()),
                Event::Failed("Envoi refusé par le site.".into())
            ]
        );
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn asks_to_pair_again_when_the_token_is_revoked() {
        let mut q = queue("revoked");
        q.add(payload("revoked-match"));
        assert_eq!(q.upload(NOW, |_| Some((401, String::new())), |_| {}), Next::Pair);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn retries_an_unconfirmed_receipt() {
        let mut q = queue("unconfirmed");
        q.add(payload("unconfirmed-match"));
        assert_eq!(q.upload(NOW, |_| receipt("another-match"), |_| {}), Next::Retry);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn replaces_a_match_sent_twice_and_keeps_twenty_at_most() {
        let mut q = queue("bounded");
        assert!(q.add(payload("match-0")));
        assert!(q.add(payload("match-0")));
        assert_eq!(q.len(), 1);
        for index in 1..MAX_QUEUE {
            assert!(q.add(payload(&format!("match-{index}"))));
        }
        assert!(!q.add(payload("one-too-many")));
        assert_eq!(q.len(), MAX_QUEUE);
        assert_eq!(q.items[0]["state"]["MatchGuid"], "match-1");
        let _ = fs::remove_file(&q.path);
    }

    #[test]
    fn forgets_matches_older_than_a_day_when_loading() {
        let mut q = queue("stale");
        q.add(payload("stale-match"));
        assert_eq!(Queue::load(q.path.clone(), NOW + 2 * 86_400_000).len(), 0);
        let _ = fs::remove_file(&q.path);
    }
}
