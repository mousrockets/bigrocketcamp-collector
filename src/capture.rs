//! Reads the Rocket League Stats API stream and keeps a match's final scoreboard.
//!
//! A port of `src/lib/live-stats.ts` in the web app (`statsStream`, `matchCapture`, `checkLiveMatch`):
//! the payload sent to `/api/live` must stay byte for byte what the browser bridge sends.

use serde_json::{Map, Value};

const MAX_BUFFER: usize = 262_144;
const MAX_MESSAGES: usize = 100;
const MAX_BLOCKED: usize = 30;
const MAX_SEEN: usize = 16;

#[derive(Debug, PartialEq, Eq)]
pub struct StreamError(pub &'static str);

/// Splits WebSocket text into JSON messages: a frame may hold several objects, or part of one.
#[derive(Default)]
pub struct StatsStream {
    buffer: String,
}

impl StatsStream {
    pub fn push(&mut self, text: &str) -> Result<Vec<Value>, StreamError> {
        if text.len() > MAX_BUFFER || self.buffer.len() + text.len() > MAX_BUFFER {
            self.buffer.clear();
            return Err(StreamError("Message local trop volumineux."));
        }
        self.buffer.push_str(text);
        match split_messages(&self.buffer) {
            Some((messages, consumed)) => {
                self.buffer.drain(..consumed);
                Ok(messages)
            }
            None => {
                self.buffer.clear();
                Err(StreamError("Message local illisible."))
            }
        }
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }
}

/// The complete messages at the start of `buffer`, and how many bytes they took; `None` if it is not JSON.
fn split_messages(buffer: &str) -> Option<(Vec<Value>, usize)> {
    let mut messages = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut consumed = 0;
    for (index, character) in buffer.char_indices() {
        let Some(from) = start else {
            if is_js_space(character) {
                consumed = index + character.len_utf8();
                continue;
            }
            if character != '{' {
                return None;
            }
            start = Some(index);
            depth = 1;
            continue;
        };
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let mut message: Value = serde_json::from_str(&buffer[from..=index]).ok()?;
                    if let Some(Value::String(data)) = message.get("Data") {
                        let data = serde_json::from_str(data).ok()?;
                        message["Data"] = data;
                    }
                    messages.push(message);
                    if messages.len() > MAX_MESSAGES {
                        return None;
                    }
                    start = None;
                    consumed = index + 1;
                }
            }
            _ => {}
        }
    }
    Some((messages, consumed))
}

/// JavaScript's `\s`, which differs from Unicode White_Space on U+0085 and U+FEFF.
fn is_js_space(character: char) -> bool {
    character == '\u{feff}' || (character.is_whitespace() && character != '\u{85}')
}

struct CurrentMatch {
    guid: String,
    started_at: i64,
    seen: Vec<Value>,
    end: Option<Value>,
    ended_at: Option<i64>,
    candidate: Option<String>,
    /// The last scoreboard of this match outside a replay with at least one identified player.
    last: Option<Value>,
    /// `TimeSeconds` and `bOvertime` as `keep_clock` keeps them.
    clock: Map<String, Value>,
    sent: bool,
}

impl CurrentMatch {
    /// The scoreboard to send when the match is left before two identical post-winner updates: after a
    /// forfeit the losers often quit at once, and the game may stop sending updates.
    fn left(self, now: i64) -> Option<Value> {
        if self.sent {
            return None;
        }
        let payload = scoreboard(&self, self.last.as_ref()?)?;
        check_live_match(&payload, now).is_ok().then_some(payload)
    }
}

/// Follows one live match from its creation and returns its final scoreboard once.
///
/// A live match must be observed from its creation or initialization: never a loaded replay, nor another
/// match's cached scoreboard. The winner always comes from `MatchEnded`. Players without a real identifier
/// (bots, players who left) are left out. Two identical post-winner updates send the scoreboard at once;
/// otherwise the last known one goes when the match is left (`MatchDestroyed`, a new match, the game closing).
#[derive(Default)]
pub struct MatchCapture {
    current: Option<CurrentMatch>,
    blocked: Vec<String>,
}

impl MatchCapture {
    pub fn handle(&mut self, message: &Value, now: i64) -> Option<Value> {
        let event = message.get("Event").and_then(Value::as_str).unwrap_or_default();
        let data = message.get("Data")?;
        let guid = data.get("MatchGuid")?.as_str()?;
        let is_current = self.current.as_ref().is_some_and(|current| current.guid == guid);
        match event {
            "ReplayCreated" => {
                if !self.blocked.iter().any(|blocked| blocked == guid) {
                    self.blocked.push(guid.to_owned());
                    if self.blocked.len() > MAX_BLOCKED {
                        self.blocked.remove(0);
                    }
                }
                if is_current {
                    self.current = None;
                }
                return None;
            }
            "MatchDestroyed" => {
                return if is_current { self.finish(now) } else { None };
            }
            _ => {}
        }
        let blocked = self.blocked.iter().any(|blocked| blocked == guid);
        if matches!(event, "MatchCreated" | "MatchInitialized") && !blocked {
            if is_current {
                return None;
            }
            let previous = self.finish(now);
            self.current = Some(CurrentMatch {
                guid: guid.to_owned(),
                started_at: now,
                seen: Vec::new(),
                end: None,
                ended_at: None,
                candidate: None,
                last: None,
                clock: Map::new(),
                sent: false,
            });
            return previous;
        }
        let current = self.current.as_mut().filter(|current| current.guid == guid && !current.sent && !blocked)?;
        if event == "MatchEnded" {
            let mut end = Map::new();
            end.insert("MatchGuid".into(), Value::String(guid.to_owned()));
            copy(&mut end, data, "WinnerTeamNum");
            current.end = Some(Value::Object(end));
            current.ended_at = Some(now);
            current.candidate = None;
            return None;
        }
        let players = data.get("Players").and_then(Value::as_array).filter(|_| event == "UpdateState")?;
        for player in players {
            let id = player.get("PrimaryId").cloned().unwrap_or(Value::Null);
            if !current.seen.contains(&id) {
                current.seen.push(id);
            }
        }
        if current.seen.len() > MAX_SEEN {
            self.current = None;
            return None;
        }
        let state = final_state(guid, data);
        // An empty update, a replay or an unreadable one never replaces the last known scoreboard.
        let usable = state.as_ref().filter(|state| {
            state["Game"]["bReplay"] == false && state["Players"].as_array().is_some_and(|players| !players.is_empty())
        });
        if let Some(state) = usable {
            current.last = Some(state.clone());
            keep_clock(&mut current.clock, &data["Game"], current.end.is_some());
        }
        current.end.as_ref()?;
        let Some(payload) = usable.and_then(|state| scoreboard(current, state)) else {
            current.candidate = None;
            return None;
        };
        if check_live_match(&payload, now).is_err() {
            current.candidate = None;
            return None;
        }
        let snapshot = payload["state"].to_string();
        if current.candidate.as_ref() != Some(&snapshot) {
            current.candidate = Some(snapshot);
            return None;
        }
        current.sent = true;
        Some(payload)
    }

    pub fn reset(&mut self) {
        self.current = None;
    }

    /// Leaves the match being followed, and returns its last known scoreboard if it ended and was not sent.
    pub fn finish(&mut self, now: i64) -> Option<Value> {
        self.current.take()?.left(now)
    }

    /// A match followed since its start and not finished yet.
    pub fn in_match(&self) -> bool {
        self.current.as_ref().is_some_and(|current| !current.sent)
    }
}

/// The payload sent to `/api/live`: the winner fields come from `MatchEnded`, and `seenIds` keeps the sent players.
fn scoreboard(current: &CurrentMatch, state: &Value) -> Option<Value> {
    let end = current.end.as_ref()?;
    let source = &state["Game"];
    let teams = source["Teams"].as_array()?;
    let winner = teams.iter().find(|team| team["TeamNum"] == end["WinnerTeamNum"])?.get("Name")?;
    let mut game = Map::new();
    copy(&mut game, source, "PlaylistId");
    copy(&mut game, source, "Teams");
    game.insert("bHasWinner".into(), Value::Bool(true));
    copy(&mut game, source, "bReplay");
    game.insert("Winner".into(), winner.clone());
    for key in ["TimeSeconds", "bOvertime"] {
        if let Some(value) = current.clock.get(key) {
            game.insert(key.into(), value.clone());
        }
    }
    let mut state = state.clone();
    state["Game"] = Value::Object(game);
    let ids: Vec<String> = state["Players"].as_array()?.iter().map(|player| canonical(&player["PrimaryId"])).collect();
    let mut seen = Vec::new();
    let mut kept = Vec::new();
    for id in &current.seen {
        let key = canonical(id);
        if ids.contains(&key) && !kept.contains(&key) {
            kept.push(key);
            seen.push(id.clone());
        }
    }
    let mut payload = Map::new();
    payload.insert("state".into(), state);
    payload.insert("end".into(), end.clone());
    payload.insert("startedAt".into(), current.started_at.into());
    payload.insert("endedAt".into(), current.ended_at.map_or(Value::Null, Value::from));
    payload.insert("seenIds".into(), Value::Array(seen));
    Some(Value::Object(payload))
}

/// The clock of the last update before `MatchEnded`, which the API reads a forfeit from. An update after the end
/// can only lower the time left: the tick that reached 0:00 may arrive after `MatchEnded`.
fn keep_clock(kept: &mut Map<String, Value>, game: &Value, ended: bool) {
    if !ended {
        kept.clear();
        copy(kept, game, "TimeSeconds");
        copy(kept, game, "bOvertime");
        return;
    }
    if let Some(seen) = game.get("TimeSeconds").filter(|seen| seen.is_number())
        && kept.get("TimeSeconds").and_then(Value::as_f64).is_none_or(|time| seen.as_f64() < Some(time))
    {
        kept.insert("TimeSeconds".into(), seen.clone());
    }
    if !kept.contains_key("bOvertime") {
        copy(kept, game, "bOvertime");
    }
}

/// The fields of an update kept in the scoreboard, in the browser's order, without the players who have no
/// real identifier; `None` when the update is unreadable. The winner fields are added by `scoreboard`.
fn final_state(guid: &str, data: &Value) -> Option<Value> {
    let players = data["Players"]
        .as_array()?
        .iter()
        .filter(|player| is_player_id(&player["PrimaryId"]))
        .map(|player| pick(player, &PLAYER_FIELDS))
        .collect();
    let mut game = Map::new();
    let source = data.get("Game").unwrap_or(&Value::Null);
    copy(&mut game, source, "PlaylistId");
    match source.get("Teams") {
        None | Some(Value::Null) => {}
        Some(Value::Array(teams)) => {
            if teams.iter().any(Value::is_null) {
                return None;
            }
            let teams = teams.iter().map(|team| pick(team, &TEAM_FIELDS)).collect();
            game.insert("Teams".into(), Value::Array(teams));
        }
        Some(_) => return None,
    }
    copy(&mut game, source, "bReplay");
    let mut state = Map::new();
    state.insert("MatchGuid".into(), Value::String(guid.to_owned()));
    state.insert("Players".into(), Value::Array(players));
    state.insert("Game".into(), Value::Object(game));
    Some(Value::Object(state))
}

const PLAYER_FIELDS: [&str; 7] = ["Name", "PrimaryId", "TeamNum", "Score", "Goals", "Assists", "Saves"];
const TEAM_FIELDS: [&str; 3] = ["Name", "TeamNum", "Score"];

fn pick(source: &Value, keys: &[&str]) -> Value {
    let mut picked = Map::new();
    for key in keys {
        copy(&mut picked, source, key);
    }
    Value::Object(picked)
}

/// Like `JSON.stringify`, a missing field stays missing instead of becoming `null`.
fn copy(target: &mut Map<String, Value>, source: &Value, key: &str) {
    if let Some(value) = source.get(key) {
        target.insert(key.to_owned(), value.clone());
    }
}

/// Public matchmaking playlists and their full scoreboard; anything else fails closed.
fn player_count(playlist: &Value) -> Option<usize> {
    let count = match integer(playlist)? {
        1 | 10 => 2,
        2 | 11 | 27 => 4,
        3 | 13 | 28 | 29 | 30 | 34 => 6,
        4 => 8,
        _ => return None,
    };
    Some(count)
}

/// `Number.isSafeInteger`.
fn integer(value: &Value) -> Option<i64> {
    let number = value.as_f64()?;
    (number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0).then_some(number as i64)
}

fn team_number(value: &Value) -> Option<i64> {
    integer(value).filter(|team| *team == 0 || *team == 1)
}

fn bounded(value: Option<&Value>) -> bool {
    value.and_then(integer).is_some_and(|number| (0..=1_000_000).contains(&number))
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The web app's identifier: PSN becomes PS4, and the Epic account id is lowercase.
fn canonical(id: &Value) -> String {
    let id = match id {
        Value::String(id) => id.clone(),
        other => other.to_string(),
    };
    if let Some(rest) = id.strip_prefix("PSN|") {
        return format!("PS4|{rest}");
    }
    if let Some(rest) = id.strip_prefix("Epic|") {
        let end = rest.find('|').unwrap_or(rest.len());
        if end > 0 {
            return format!("Epic|{}{}", rest[..end].to_lowercase(), &rest[end..]);
        }
    }
    id
}

/// `^(Epic|Steam|PS4|PSN|XboxOne|Switch|PsyNet)\|[^\s|]{1,128}\|0$`: a real player, never a bot.
fn is_player_id(id: &Value) -> bool {
    let Some((platform, rest)) = id.as_str().and_then(|id| id.split_once('|')) else {
        return false;
    };
    let Some(account) = rest.strip_suffix("|0") else {
        return false;
    };
    matches!(platform, "Epic" | "Steam" | "PS4" | "PSN" | "XboxOne" | "Switch" | "PsyNet")
        && (1..=128).contains(&utf16_len(account))
        && account.chars().all(|character| character != '|' && !is_js_space(character))
}

fn unique(values: impl Iterator<Item = String>) -> usize {
    let mut seen: Vec<String> = values.collect();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

/// The checks the API runs on a scoreboard, so that a match it would refuse is neither kept nor sent.
pub fn check_live_match(payload: &Value, now: i64) -> Result<(), &'static str> {
    let state = &payload["state"];
    let end = &payload["end"];
    let game = &state["Game"];
    let guid = state["MatchGuid"].as_str().unwrap_or_default();
    let winner_team = team_number(&end["WinnerTeamNum"]);
    if guid.trim().is_empty()
        || utf16_len(guid) > 128
        || end["MatchGuid"].as_str() != Some(guid)
        || winner_team.is_none()
    {
        return Err("Fin de match absente ou incohérente.");
    }
    let (Some(started), Some(ended)) = (integer(&payload["startedAt"]), integer(&payload["endedAt"])) else {
        return Err("Date du match invalide.");
    };
    if started > ended || ended - started > 10_800_000 || ended > now + 30_000 || ended < now - 86_400_000 {
        return Err("Date du match invalide.");
    }
    let count = player_count(&game["PlaylistId"]);
    let Some(count) = count.filter(|_| game["bHasWinner"] == true && game["bReplay"] == false) else {
        return Err("Match public terminé requis.");
    };
    let teams = game["Teams"].as_array().map(Vec::as_slice).unwrap_or_default();
    let valid_team = |team: &Value| {
        team_number(&team["TeamNum"]).is_some()
            && team["Name"].as_str().is_some_and(|name| !name.is_empty())
            && integer(&team["Score"]).is_some_and(|score| score >= 0)
    };
    if teams.len() != 2
        || team_number(&teams[0]["TeamNum"]) == team_number(&teams[1]["TeamNum"])
        || !teams.iter().all(valid_team)
    {
        return Err("Équipes finales invalides.");
    }
    let winner = teams.iter().find(|team| team_number(&team["TeamNum"]) == winner_team);
    if winner.map(|team| &team["Name"]) != Some(&game["Winner"]) {
        return Err("Gagnant du tableau différent de la fin de match.");
    }
    // After a forfeit the losers often leave at once: the scoreboard keeps whoever is still identified.
    let players = state["Players"].as_array().map(Vec::as_slice).unwrap_or_default();
    let seen = payload["seenIds"].as_array().map(Vec::as_slice).unwrap_or_default();
    if players.is_empty() || players.len() > count || seen.len() != players.len() || !payload["seenIds"].is_array() {
        return Err("Aucun joueur identifié dans le tableau final.");
    }
    let ids: Vec<String> = players.iter().map(|player| canonical(&player["PrimaryId"])).collect();
    if unique(ids.iter().cloned()) != players.len()
        || unique(seen.iter().map(canonical)) != players.len()
        || !seen.iter().all(|id| ids.contains(&canonical(id)))
    {
        return Err("Participants du tableau incomplets ou ambigus.");
    }
    if !players.iter().all(|player| is_player_id(&player["PrimaryId"])) {
        return Err("Identité de joueur manquante ou bot.");
    }
    let on_team = |team: i64| players.iter().filter(|player| integer(&player["TeamNum"]) == Some(team)).count();
    if on_team(0) + on_team(1) != players.len() || on_team(0) > count / 2 || on_team(1) > count / 2 {
        return Err("Tableau des équipes incohérent.");
    }
    for (player, id) in players.iter().zip(&ids) {
        if id.is_empty() || utf16_len(id) > 256 {
            return Err("Identifiants invalides.");
        }
        if !["Score", "Goals", "Assists", "Saves"].iter().all(|key| bounded(player.get(*key))) {
            return Err("Statistiques invalides.");
        }
        let name = player["Name"].as_str().unwrap_or_default();
        if name.trim().is_empty() || utf16_len(name) > 128 {
            return Err("Nom du joueur invalide.");
        }
    }
    Ok(())
}

/// The match identifier the API confirms in its receipt.
pub fn receipt_id(payload: &Value) -> String {
    payload["state"]["MatchGuid"].as_str().unwrap_or_default().trim().to_lowercase()
}

#[cfg(test)]
pub mod tests_support {
    use serde_json::{Value, json};

    pub const NOW: i64 = 1_793_000_000_000;

    /// A synthetic 3v3 scoreboard: fictitious players only.
    pub fn payload(guid: &str) -> Value {
        let account = "b".repeat(32);
        let identities = [
            ("renamed-player".to_owned(), format!("Epic|{account}|0")),
            ("Friend One".to_owned(), format!("Epic|{}|0", "c".repeat(32))),
            ("Friend Two".to_owned(), "PS4|222222|0".to_owned()),
            ("Opponent 1".to_owned(), format!("Epic|{}|0", "d".repeat(32))),
            ("Opponent 2".to_owned(), format!("Epic|{}|0", "e".repeat(32))),
            ("Opponent 3".to_owned(), format!("Epic|{}|0", "f".repeat(32))),
        ];
        let players: Vec<Value> = identities
            .iter()
            .enumerate()
            .map(|(i, (name, id))| {
                json!({ "Name": name, "PrimaryId": id, "TeamNum": if i < 3 { 0 } else { 1 },
                        "Score": 100 + i * 10, "Goals": 0, "Assists": 0, "Saves": 1 })
            })
            .collect();
        let seen: Vec<Value> = players.iter().map(|player| player["PrimaryId"].clone()).collect();
        json!({
            "state": {
                "MatchGuid": guid,
                "Players": players,
                "Game": {
                    "PlaylistId": 13,
                    "Teams": [{ "Name": "Blue", "TeamNum": 0, "Score": 1 }, { "Name": "Orange", "TeamNum": 1, "Score": 2 }],
                    "bHasWinner": true,
                    "bReplay": false,
                    "Winner": "Orange"
                }
            },
            "end": { "MatchGuid": guid, "WinnerTeamNum": 1 },
            "startedAt": NOW - 360_000,
            "endedAt": NOW - 1000,
            "seenIds": seen
        })
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{NOW, payload};
    use super::*;
    use serde_json::json;

    fn packet(event: &str, data: Value) -> Value {
        json!({ "Event": event, "Data": data })
    }

    fn dated(mut p: Value) -> Value {
        p["startedAt"] = json!(NOW);
        p["endedAt"] = json!(NOW);
        p
    }

    /// A state seen while the match is played: no winner yet.
    fn playing(p: &Value) -> Value {
        let mut state = p["state"].clone();
        state["Game"]["bHasWinner"] = json!(false);
        state["Game"]["Winner"] = json!("");
        state
    }

    fn keep_players(state: &Value, count: usize) -> Value {
        let mut state = state.clone();
        state["Players"].as_array_mut().unwrap().truncate(count);
        state
    }

    fn finish(capture: &mut MatchCapture, p: &Value, received: &mut Vec<Value>) {
        for message in [packet("MatchEnded", p["end"].clone()), packet("UpdateState", p["state"].clone())] {
            received.extend(capture.handle(&message, NOW));
        }
        received.extend(capture.handle(&packet("UpdateState", p["state"].clone()), NOW));
    }

    #[test]
    fn official_stream_concatenated_fragmented_encoded_and_escaped() {
        let frames = [
            packet("UpdateState", json!({ "MatchGuid": "synthetic", "Name": "Nom \"{ }\" \\ é" })),
            packet("MatchEnded", Value::String(json!({ "MatchGuid": "synthetic", "WinnerTeamNum": 1 }).to_string())),
        ];
        let text = frames.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
        let mut stream = StatsStream::default();
        let mut out = Vec::new();
        let characters: Vec<char> = text.chars().collect();
        for chunk in characters.chunks(7) {
            out.extend(stream.push(&chunk.iter().collect::<String>()).unwrap());
        }
        let decoded = packet("MatchEnded", json!({ "MatchGuid": "synthetic", "WinnerTeamNum": 1 }));
        assert_eq!(out, [frames[0].clone(), decoded]);
        assert_eq!(stream.push("  "), Ok(vec![]));
        assert!(stream.push("x").is_err());
        assert!(stream.push(&"x".repeat(262_145)).is_err());
        assert_eq!(stream.push(&frames[0].to_string()), Ok(vec![frames[0].clone()]));
    }

    #[test]
    fn waits_for_two_consistent_final_scoreboards() {
        let mut received = Vec::new();
        let p = payload("synthetic-live-match");
        let guid = json!({ "MatchGuid": "synthetic-live-match" });
        let mut capture = MatchCapture::default();
        capture.handle(&packet("MatchCreated", guid.clone()), NOW);
        capture.handle(&packet("MatchInitialized", guid), NOW);
        let mut pre = p["state"].clone();
        pre["Game"]["bHasWinner"] = json!(false);
        pre["Game"]["Winner"] = json!("");
        pre["Game"]["bReplay"] = json!(true);
        received.extend(capture.handle(&packet("UpdateState", pre.clone()), NOW));
        received.extend(capture.handle(&packet("MatchEnded", p["end"].clone()), NOW));
        received.extend(capture.handle(&packet("UpdateState", pre), NOW));
        assert!(received.is_empty());
        received.extend(capture.handle(&packet("UpdateState", p["state"].clone()), NOW));
        assert!(received.is_empty());
        let mut last = p["state"].clone();
        last["Players"][0]["Score"] = json!(250);
        received.extend(capture.handle(&packet("UpdateState", last.clone()), NOW));
        assert!(received.is_empty());
        received.extend(capture.handle(&packet("UpdateState", last), NOW));
        assert_eq!(received.len(), 1);
        assert_eq!(received[0]["state"]["Players"][0]["Score"], 250);
        assert_eq!(received[0]["startedAt"], NOW);
        finish(&mut capture, &p, &mut received);
        assert_eq!(received.len(), 1);
    }

    #[test]
    fn sends_the_same_json_as_the_browser() {
        let mut received = Vec::new();
        let p = payload("same-json");
        let mut capture = MatchCapture::default();
        capture.handle(&packet("MatchCreated", json!({ "MatchGuid": "same-json" })), NOW);
        let mut noisy = p["state"].clone();
        noisy["Players"][0]["Touches"] = json!(12);
        noisy["Game"]["Clock"] = json!("0:00");
        noisy["Game"]["Teams"][0]["ColorPrimary"] = json!("1873FF");
        received.extend(capture.handle(&packet("MatchEnded", p["end"].clone()), NOW));
        received.extend(capture.handle(&packet("UpdateState", noisy.clone()), NOW));
        received.extend(capture.handle(&packet("UpdateState", noisy), NOW));
        let player = r#"{"Name":"renamed-player","PrimaryId":"Epic|bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb|0","TeamNum":0,"Score":100,"Goals":0,"Assists":0,"Saves":1}"#;
        let teams = r#"[{"Name":"Blue","TeamNum":0,"Score":1},{"Name":"Orange","TeamNum":1,"Score":2}]"#;
        let body = received[0].to_string();
        assert!(body.starts_with(&format!(r#"{{"state":{{"MatchGuid":"same-json","Players":[{player},"#)), "{body}");
        assert!(body.contains(&format!(
            r#""Game":{{"PlaylistId":13,"Teams":{teams},"bHasWinner":true,"bReplay":false,"Winner":"Orange"}}}},"end":{{"MatchGuid":"same-json","WinnerTeamNum":1}},"startedAt":{NOW},"endedAt":{NOW},"seenIds":["Epic|"#
        )), "{body}");
        assert_eq!(body, dated(p).to_string());
    }

    #[test]
    fn never_imports_a_replay_a_mid_match_connection_or_a_vanished_scoreboard() {
        for mode in ["replay", "mid-match", "destroyed"] {
            let mut received = Vec::new();
            let p = payload(mode);
            let mut capture = MatchCapture::default();
            let guid = json!({ "MatchGuid": mode });
            if mode == "replay" {
                capture.handle(&packet("ReplayCreated", guid.clone()), NOW);
            }
            if mode != "mid-match" {
                capture.handle(&packet("MatchCreated", guid.clone()), NOW);
            }
            received.extend(capture.handle(&packet("UpdateState", p["state"].clone()), NOW));
            if mode == "destroyed" {
                capture.handle(&packet("MatchDestroyed", guid), NOW);
            }
            finish(&mut capture, &p, &mut received);
            assert!(received.is_empty(), "{mode}");
        }
    }

    #[test]
    fn validation_refuses_bots_private_unknown_winner_duplicates_and_partial_snapshots() {
        let mut forfeited = payload("forfeit");
        forfeited["state"]["Game"]["Teams"][0]["Score"] = json!(5);
        assert_eq!(check_live_match(&forfeited, NOW), Ok(()));
        let mut alone = payload("alone");
        alone["state"]["Players"].as_array_mut().unwrap().truncate(1);
        alone["seenIds"].as_array_mut().unwrap().truncate(1);
        assert_eq!(check_live_match(&alone, NOW), Ok(()));
        let mutations: Vec<fn(&mut Value)> = vec![
            |p| p["end"]["WinnerTeamNum"] = json!(2),
            |p| p["end"]["MatchGuid"] = json!("other"),
            |p| p["state"]["Game"]["bHasWinner"] = json!(false),
            |p| p["state"]["Game"]["bReplay"] = json!(true),
            |p| p["state"]["Game"]["Winner"] = json!("Blue"),
            |p| p["state"]["Game"]["PlaylistId"] = json!(6),
            |p| p["state"]["Game"]["PlaylistId"] = json!(40),
            |p| _ = p["state"]["Players"].as_array_mut().unwrap().pop(),
            |p| _ = p["seenIds"].as_array_mut().unwrap().pop(),
            |p| p["state"]["Players"][1]["PrimaryId"] = p["state"]["Players"][0]["PrimaryId"].clone(),
            |p| p["state"]["Players"][0]["PrimaryId"] = json!("Bot|1|0"),
            |p| p["state"]["Players"][0]["Score"] = json!(1.5),
            |p| p["state"]["Players"][0]["Score"] = json!(-1),
            |p| p["state"]["Players"][0]["Name"] = json!("  "),
            |p| p["endedAt"] = json!(NOW + 60_000),
            |p| p["startedAt"] = json!(NOW + 1),
            |p| {
                p["state"]["Players"] = json!([]);
                p["seenIds"] = json!([]);
            },
            |p| p["state"]["Players"][3]["TeamNum"] = json!(0),
            |p| p["state"]["Players"][3]["TeamNum"] = json!(2),
            |p| p["seenIds"][0] = json!("Epic|someone-else|0"),
        ];
        for (index, mutate) in mutations.iter().enumerate() {
            let mut p = payload("mutated");
            mutate(&mut p);
            assert!(check_live_match(&p, NOW).is_err(), "mutation {index}");
        }
    }

    #[test]
    fn a_forfeit_counts_with_the_players_still_there() {
        let p = payload("forfeit");
        let guid = json!({ "MatchGuid": "forfeit" });
        let end = json!({ "MatchGuid": "forfeit", "WinnerTeamNum": 0 });
        // The opponents leave at once, then a teammate: the last scoreboard goes when the match is left.
        let mut capture = MatchCapture::default();
        let mut received = Vec::new();
        capture.handle(&packet("MatchCreated", guid.clone()), NOW);
        capture.handle(&packet("UpdateState", playing(&p)), NOW);
        capture.handle(&packet("MatchEnded", end.clone()), NOW);
        received.extend(capture.handle(&packet("UpdateState", keep_players(&playing(&p), 3)), NOW));
        received.extend(capture.handle(&packet("UpdateState", keep_players(&playing(&p), 2)), NOW));
        assert!(received.is_empty());
        assert!(capture.in_match());
        received.extend(capture.handle(&packet("MatchDestroyed", guid.clone()), NOW));
        assert_eq!(received.len(), 1);
        let sent = &received[0];
        assert_eq!(sent["state"]["Players"], keep_players(&p["state"], 2)["Players"]);
        assert_eq!(sent["seenIds"], json!(p["seenIds"].as_array().unwrap()[..2]));
        assert_eq!(sent["state"]["Game"]["bHasWinner"], true);
        assert_eq!(sent["state"]["Game"]["Winner"], "Blue");
        assert_eq!(sent["end"], end);
        assert_eq!(check_live_match(sent, NOW), Ok(()));
        assert!(!capture.in_match());

        // Two identical updates without the opponents send the scoreboard at once.
        let mut capture = MatchCapture::default();
        let mut received = Vec::new();
        capture.handle(&packet("MatchCreated", guid), NOW);
        capture.handle(&packet("UpdateState", playing(&p)), NOW);
        capture.handle(&packet("MatchEnded", end), NOW);
        for _ in 0..2 {
            received.extend(capture.handle(&packet("UpdateState", keep_players(&playing(&p), 3)), NOW));
        }
        assert_eq!(received.len(), 1);
        assert_eq!(received[0]["state"]["Players"].as_array().unwrap().len(), 3);
        assert_eq!(received[0]["seenIds"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_bot_in_place_of_an_opponent_is_left_out() {
        for bot in [json!("Unknown|0|0"), json!(""), Value::Null] {
            let p = payload("bot");
            let mut capture = MatchCapture::default();
            let mut received = Vec::new();
            capture.handle(&packet("MatchCreated", json!({ "MatchGuid": "bot" })), NOW);
            capture.handle(&packet("UpdateState", p["state"].clone()), NOW);
            let mut replaced = p["state"].clone();
            replaced["Players"][5]["Name"] = json!("Bot");
            replaced["Players"][5]["PrimaryId"] = bot.clone();
            capture.handle(&packet("MatchEnded", p["end"].clone()), NOW);
            for _ in 0..2 {
                received.extend(capture.handle(&packet("UpdateState", replaced.clone()), NOW));
            }
            assert_eq!(received.len(), 1, "{bot}");
            assert_eq!(received[0]["state"]["Players"], keep_players(&p["state"], 5)["Players"]);
            assert_eq!(received[0]["seenIds"], json!(p["seenIds"].as_array().unwrap()[..5]));
        }
        // A missing player entry is ignored too, instead of dropping the match.
        let p = payload("null-player");
        let mut capture = MatchCapture::default();
        capture.handle(&packet("MatchCreated", json!({ "MatchGuid": "null-player" })), NOW);
        let mut with_null = p["state"].clone();
        with_null["Players"][5] = Value::Null;
        capture.handle(&packet("MatchEnded", p["end"].clone()), NOW);
        capture.handle(&packet("UpdateState", with_null.clone()), NOW);
        let sent = capture.handle(&packet("UpdateState", with_null), NOW).unwrap();
        assert_eq!(sent["state"]["Players"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn sends_the_last_known_scoreboard_when_no_update_follows_the_end() {
        let p = payload("no-update");
        let guid = json!({ "MatchGuid": "no-update" });
        let start = |capture: &mut MatchCapture| {
            capture.handle(&packet("MatchCreated", guid.clone()), NOW);
            capture.handle(&packet("UpdateState", playing(&p)), NOW);
            capture.handle(&packet("MatchEnded", p["end"].clone()), NOW)
        };
        // The winner comes from MatchEnded: the scoreboard is the complete one, byte for byte.
        let mut capture = MatchCapture::default();
        assert_eq!(start(&mut capture), None);
        let sent = capture.handle(&packet("MatchDestroyed", guid.clone()), NOW).unwrap();
        assert_eq!(sent.to_string(), dated(p.clone()).to_string());

        // Empty or replayed updates after the end are skipped, then the game closes.
        let mut capture = MatchCapture::default();
        start(&mut capture);
        let mut replay = p["state"].clone();
        replay["Game"]["bReplay"] = json!(true);
        assert_eq!(capture.handle(&packet("UpdateState", keep_players(&p["state"], 0)), NOW), None);
        assert_eq!(capture.handle(&packet("UpdateState", replay), NOW), None);
        assert_eq!(capture.finish(NOW).map(|sent| sent.to_string()), Some(dated(p.clone()).to_string()));

        // The next match starts first: the previous one is sent, and the new one is followed.
        let mut capture = MatchCapture::default();
        start(&mut capture);
        let sent = capture.handle(&packet("MatchCreated", json!({ "MatchGuid": "next" })), NOW);
        assert_eq!(sent.map(|sent| sent.to_string()), Some(dated(p.clone()).to_string()));
        assert!(capture.in_match());

        // Without MatchEnded, leaving sends nothing.
        let mut capture = MatchCapture::default();
        capture.handle(&packet("MatchCreated", guid.clone()), NOW);
        capture.handle(&packet("UpdateState", playing(&p)), NOW);
        assert_eq!(capture.handle(&packet("MatchDestroyed", guid), NOW), None);
    }

    /// A match whose last update before the end shows 143 seconds left, then `after` once it ended.
    fn clock_sent(after: Value) -> Value {
        let p = payload("clock");
        let mut capture = MatchCapture::default();
        let mut received = Vec::new();
        capture.handle(&packet("MatchCreated", json!({ "MatchGuid": "clock" })), NOW);
        let mut before = p["state"].clone();
        before["Game"]["TimeSeconds"] = json!(143);
        before["Game"]["bOvertime"] = json!(false);
        capture.handle(&packet("UpdateState", before), NOW);
        capture.handle(&packet("MatchEnded", p["end"].clone()), NOW);
        let mut last = p["state"].clone();
        for (key, value) in after.as_object().unwrap() {
            last["Game"][key] = value.clone();
        }
        for _ in 0..2 {
            received.extend(capture.handle(&packet("UpdateState", last.clone()), NOW));
        }
        assert_eq!(received.len(), 1);
        received.remove(0)
    }

    #[test]
    fn sends_the_clock_of_the_last_update_before_the_end() {
        let sent = clock_sent(json!({ "TimeSeconds": 300, "bOvertime": false }));
        let game = r#""bHasWinner":true,"bReplay":false,"Winner":"Orange","TimeSeconds":143,"bOvertime":false}"#;
        assert!(sent.to_string().contains(game), "{sent}");
        assert_eq!(check_live_match(&sent, NOW), Ok(()));
        // The tick that reached 0:00 after MatchEnded lowers the time left; overtime stays as it was before the end.
        let sent = clock_sent(json!({ "TimeSeconds": 0, "bOvertime": true }));
        assert_eq!(sent["state"]["Game"]["TimeSeconds"], 0);
        assert_eq!(sent["state"]["Game"]["bOvertime"], false);
        // An update after the end without the clock keeps the one before it.
        let sent = clock_sent(json!({}));
        assert_eq!(sent["state"]["Game"]["TimeSeconds"], 143);
        // A game that sends no clock at all: the scoreboard has none either, byte for byte as before.
        let mut capture = MatchCapture::default();
        let p = payload("no-clock");
        capture.handle(&packet("MatchCreated", json!({ "MatchGuid": "no-clock" })), NOW);
        capture.handle(&packet("UpdateState", playing(&p)), NOW);
        capture.handle(&packet("MatchEnded", p["end"].clone()), NOW);
        let sent = capture.handle(&packet("MatchDestroyed", json!({ "MatchGuid": "no-clock" })), NOW).unwrap();
        assert_eq!(sent.to_string(), dated(p).to_string());
    }

    #[test]
    fn a_forfeit_sends_the_time_left_when_the_match_is_left() {
        let p = payload("forfeit-clock");
        let guid = json!({ "MatchGuid": "forfeit-clock" });
        let mut capture = MatchCapture::default();
        capture.handle(&packet("MatchCreated", guid.clone()), NOW);
        let mut before = playing(&p);
        before["Game"]["TimeSeconds"] = json!(97);
        before["Game"]["bOvertime"] = json!(false);
        capture.handle(&packet("UpdateState", before), NOW);
        capture.handle(&packet("MatchEnded", p["end"].clone()), NOW);
        let sent = capture.handle(&packet("MatchDestroyed", guid), NOW).unwrap();
        assert_eq!(sent["state"]["Game"]["TimeSeconds"], 97);
        assert_eq!(sent["state"]["Game"]["bOvertime"], false);
    }

    #[test]
    fn canonical_ids_match_the_web_app() {
        assert_eq!(canonical(&json!("PSN|abc|0")), "PS4|abc|0");
        assert_eq!(canonical(&json!("Epic|ABCdef|0")), "Epic|abcdef|0");
        assert_eq!(canonical(&json!("Steam|ABC|0")), "Steam|ABC|0");
        assert!(is_player_id(&json!("PsyNet|x|0")));
        assert!(!is_player_id(&json!("Epic|a b|0")));
        assert!(!is_player_id(&json!("Epic||0")));
    }

    #[test]
    fn receipt_id_is_the_trimmed_lowercase_guid() {
        assert_eq!(receipt_id(&payload(" ABC-Def ")), "abc-def");
    }
}
