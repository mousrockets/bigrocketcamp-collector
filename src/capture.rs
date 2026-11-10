//! Reads the Rocket League Stats API stream and keeps a match's final scoreboard.
//!
//! A port of `src/lib/live-stats.ts` in the web app (`statsStream`, `matchCapture`, `checkLiveMatch`):
//! the payload sent to `/api/live` must stay byte for byte what the browser bridge sends.

use serde_json::{Map, Value};

const MAX_BUFFER: usize = 262_144;
const MAX_MESSAGES: usize = 100;
const MAX_BLOCKED: usize = 30;
const MAX_SEEN: usize = 16;
const MAX_DEPARTURES: usize = 8;

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
    /// The clock's changes around `MatchEnded`, as `trace_clock` keeps them.
    trace: Vec<Value>,
    /// Every identified player of the usable updates before `MatchEnded`, first seen first, with his last values.
    players: Vec<Value>,
    /// The canonical ids of the last usable update before `MatchEnded`: whoever is missing from it left.
    present: Vec<String>,
    sent: bool,
    /// Each identified player's `PrimaryId` by the shortcut and team the game names him with in its events.
    shortcuts: Vec<(String, String)>,
    /// The crossbar or post hits of each identified player before `MatchEnded`, by his `PrimaryId`.
    crossbars: Vec<(String, u64)>,
}

impl CurrentMatch {
    /// What to send when the match is left. After its end and before two identical post-winner updates, the
    /// scoreboard: after a forfeit the losers often quit at once, and the game may stop sending updates. Before its
    /// end, the left payload when the game itself left the match (`by_game`): the player left, which the API counts as
    /// a defeat. A closed socket or a stopped collector says nothing about the player, so it sends no departure.
    fn left(self, now: i64, by_game: bool) -> Option<Value> {
        if self.sent {
            return None;
        }
        let state = self.last.as_ref()?;
        let payload = match self.end {
            Some(_) => scoreboard(&self, state)?,
            None if by_game => left_match(&self, state, now)?,
            None => return None,
        };
        check_live_match(&payload, now).is_ok().then_some(payload)
    }
}

/// Follows one live match from its creation and returns its final scoreboard once.
///
/// A live match must be observed from its creation or initialization: never a loaded replay, nor another
/// match's cached scoreboard. The winner always comes from `MatchEnded`. Players without a real identifier
/// (bots, players who left) are left out. Two identical post-winner updates send the scoreboard at once;
/// otherwise the last known one goes when the match is left (`MatchDestroyed`, a new match, the game closing).
/// A match the game leaves before `MatchEnded` (`MatchDestroyed`, a new match) returns the left payload instead,
/// built from its last known scoreboard; the game closing then sends nothing.
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
                return if is_current { self.leave(now, true) } else { None };
            }
            _ => {}
        }
        let blocked = self.blocked.iter().any(|blocked| blocked == guid);
        if matches!(event, "MatchCreated" | "MatchInitialized") && !blocked {
            if is_current {
                return None;
            }
            let previous = self.leave(now, true);
            self.current = Some(CurrentMatch {
                guid: guid.to_owned(),
                started_at: now,
                seen: Vec::new(),
                end: None,
                ended_at: None,
                candidate: None,
                last: None,
                clock: Map::new(),
                trace: Vec::new(),
                players: Vec::new(),
                present: Vec::new(),
                sent: false,
                shortcuts: Vec::new(),
                crossbars: Vec::new(),
            });
            return previous;
        }
        let current = self.current.as_mut().filter(|current| current.guid == guid && !current.sent && !blocked)?;
        if event == "CrossbarHit" {
            if current.end.is_none() {
                count_crossbar(current, data);
            }
            return None;
        }
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
        remember_shortcuts(current, players);
        let state = final_state(guid, data, &current.crossbars);
        // An empty update, a replay or an unreadable one never replaces the last known scoreboard.
        let usable = state.as_ref().filter(|state| {
            state["Game"]["bReplay"] == false && state["Players"].as_array().is_some_and(|players| !players.is_empty())
        });
        if let Some(state) = usable {
            current.last = Some(state.clone());
            keep_clock(&mut current.clock, &data["Game"], current.end.is_some());
            trace_clock(&mut current.trace, &data["Game"], current.end.is_some());
            if current.end.is_none() {
                remember_players(current, state);
            }
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

    /// Stops following the match because the game closed or the collector stops: its scoreboard if it ended and was
    /// not sent, never a left payload.
    pub fn finish(&mut self, now: i64) -> Option<Value> {
        self.leave(now, false)
    }

    fn leave(&mut self, now: i64, by_game: bool) -> Option<Value> {
        self.current.take()?.left(now, by_game)
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
    let seen = seen_ids(current, &state)?;
    let mut payload = Map::new();
    payload.insert("state".into(), state);
    payload.insert("end".into(), end.clone());
    payload.insert("startedAt".into(), current.started_at.into());
    payload.insert("endedAt".into(), current.ended_at.map_or(Value::Null, Value::from));
    payload.insert("seenIds".into(), seen);
    if !current.trace.is_empty() {
        payload.insert("clockTrace".into(), Value::Array(current.trace.clone()));
    }
    let departures = departures(current);
    if !departures.is_empty() {
        payload.insert("departures".into(), Value::Array(departures));
    }
    Some(Value::Object(payload))
}

/// The payload sent to `/api/live` when the player leaves before `MatchEnded`: the last known scoreboard, without any
/// winner. The API reads only the sender's own departure from it, the match not being over.
fn left_match(current: &CurrentMatch, state: &Value, now: i64) -> Option<Value> {
    let mut payload = Map::new();
    payload.insert("left".into(), Value::Bool(true));
    payload.insert("state".into(), state.clone());
    payload.insert("startedAt".into(), current.started_at.into());
    payload.insert("leftAt".into(), now.into());
    payload.insert("seenIds".into(), seen_ids(current, state)?);
    Some(Value::Object(payload))
}

/// The ids of the scoreboard's players, in the order the collector saw them.
fn seen_ids(current: &CurrentMatch, state: &Value) -> Option<Value> {
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
    Some(Value::Array(seen))
}

/// Remembers the players of a usable update before `MatchEnded`, and which of them are still there.
fn remember_players(current: &mut CurrentMatch, state: &Value) {
    let players = state["Players"].as_array().map(Vec::as_slice).unwrap_or_default();
    current.present = players.iter().map(|player| canonical(&player["PrimaryId"])).collect();
    for player in players {
        let id = canonical(&player["PrimaryId"]);
        match current.players.iter_mut().find(|known| canonical(&known["PrimaryId"]) == id) {
            Some(known) => *known = player.clone(),
            None => current.players.push(player.clone()),
        }
    }
}

/// The players who left before `MatchEnded`, with their last values: the API counts a defeat for a club player who
/// left, and a forfeit when every opponent did. Leaving during the podium changes nothing, so a player missing only
/// after the end is not one of them.
fn departures(current: &CurrentMatch) -> Vec<Value> {
    current
        .players
        .iter()
        .filter(|player| !current.present.contains(&canonical(&player["PrimaryId"])))
        .take(MAX_DEPARTURES)
        .map(|player| pick(player, &DEPARTURE_FIELDS))
        .collect()
}

/// The clock of the last update before `MatchEnded`, which the API reads a forfeit from. The tick that reaches 0:00
/// may arrive after `MatchEnded`, so an update after the end lowers a clock that showed one second at most. More time
/// left stays: the game may set the clock to 0:00 once a team forfeits, and the forfeit would be lost.
fn keep_clock(kept: &mut Map<String, Value>, game: &Value, ended: bool) {
    if !ended {
        kept.clear();
        copy(kept, game, "TimeSeconds");
        copy(kept, game, "bOvertime");
        return;
    }
    let kept_time = kept.get("TimeSeconds").and_then(Value::as_f64);
    if let Some(seen) = game.get("TimeSeconds").filter(|seen| seen.is_number())
        && kept_time.is_none_or(|time| time <= 1.0 && seen.as_f64() < Some(time))
    {
        kept.insert("TimeSeconds".into(), seen.clone());
    }
    if !kept.contains_key("bOvertime") {
        copy(kept, game, "bOvertime");
    }
}

/// How many changes of the clock the trace keeps on each side of `MatchEnded`.
const CLOCK_TRACE_SIDE: usize = 6;

/// Adds an update's clock to the trace when it changed: the last changes before the end, the first ones after. The
/// game's way of ending a forfeit is not documented, and the API keeps the trace to show it. A game that sends no
/// clock leaves the trace empty, and the payload without it.
fn trace_clock(trace: &mut Vec<Value>, game: &Value, ended: bool) {
    let mut sample = Map::new();
    if let Some(time) = game.get("TimeSeconds").filter(|time| time.is_number()) {
        sample.insert("TimeSeconds".into(), time.clone());
    }
    if let Some(overtime) = game.get("bOvertime").filter(|overtime| overtime.is_boolean()) {
        sample.insert("bOvertime".into(), overtime.clone());
    }
    if sample.is_empty() {
        return;
    }
    sample.insert("ended".into(), Value::Bool(ended));
    let sample = Value::Object(sample);
    if trace.last() == Some(&sample) {
        return;
    }
    if !ended {
        trace.push(sample);
        if trace.len() > CLOCK_TRACE_SIDE {
            trace.remove(0);
        }
    } else if trace.iter().filter(|kept| kept["ended"] == true).count() < CLOCK_TRACE_SIDE {
        trace.push(sample);
    }
}

/// The game names a player in its events by a shortcut, unique within his team: `JSON.stringify` of both, as the app.
fn shortcut_key(shortcut: Option<&Value>, team: Option<&Value>) -> String {
    Value::Array(vec![shortcut.cloned().unwrap_or(Value::Null), team.cloned().unwrap_or(Value::Null)]).to_string()
}

/// Remembers how the game names each identified player of an update in its events.
fn remember_shortcuts(current: &mut CurrentMatch, players: &[Value]) {
    for player in players.iter().filter(|player| is_player_id(&player["PrimaryId"])) {
        let key = shortcut_key(player.get("Shortcut"), player.get("TeamNum"));
        let id = player["PrimaryId"].as_str().unwrap_or_default().to_owned();
        match current.shortcuts.iter_mut().find(|(known, _)| *known == key) {
            Some(entry) => entry.1 = id,
            None => current.shortcuts.push((key, id)),
        }
    }
}

/// A `CrossbarHit` counts for the identified player who last touched the ball; nobody known, for nobody.
fn count_crossbar(current: &mut CurrentMatch, data: &Value) {
    let touch = &data["BallLastTouch"]["Player"];
    let key = shortcut_key(touch.get("Shortcut"), touch.get("TeamNum"));
    let Some((_, id)) = current.shortcuts.iter().find(|(known, _)| *known == key) else {
        return;
    };
    let id = id.clone();
    match current.crossbars.iter_mut().find(|(known, _)| *known == id) {
        Some(entry) => entry.1 += 1,
        None => current.crossbars.push((id, 1)),
    }
}

/// The fields of an update kept in the scoreboard, in the browser's order, without the players who have no
/// real identifier; `None` when the update is unreadable. The winner fields are added by `scoreboard`.
fn final_state(guid: &str, data: &Value, crossbars: &[(String, u64)]) -> Option<Value> {
    let players = data["Players"]
        .as_array()?
        .iter()
        .filter(|player| is_player_id(&player["PrimaryId"]))
        .map(|player| {
            let mut picked = pick(player, &PLAYER_FIELDS);
            let id = player["PrimaryId"].as_str().unwrap_or_default();
            let hits = crossbars.iter().find(|(known, _)| known == id).map_or(0, |(_, hits)| *hits);
            picked["CrossbarHits"] = hits.into();
            picked
        })
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

/// `CrossbarHits` follows them: `final_state` counts it.
const PLAYER_FIELDS: [&str; 8] = ["Name", "PrimaryId", "TeamNum", "Score", "Goals", "Assists", "Saves", "Demos"];
const TEAM_FIELDS: [&str; 3] = ["Name", "TeamNum", "Score"];
const DEPARTURE_FIELDS: [&str; 9] =
    ["PrimaryId", "Name", "TeamNum", "Score", "Goals", "Assists", "Saves", "Demos", "CrossbarHits"];

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

/// The checks the API runs on a scoreboard or a left payload, so that one it would refuse is neither kept nor sent.
pub fn check_live_match(payload: &Value, now: i64) -> Result<(), &'static str> {
    if payload.get("left").is_some() { check_left_match(payload, now) } else { check_ended_match(payload, now) }
}

fn check_ended_match(payload: &Value, now: i64) -> Result<(), &'static str> {
    let state = &payload["state"];
    let end = &payload["end"];
    let game = &state["Game"];
    let guid = match_guid(state);
    let winner_team = team_number(&end["WinnerTeamNum"]);
    if guid.is_none() || end["MatchGuid"].as_str() != guid || winner_team.is_none() {
        return Err("Fin de match absente ou incohérente.");
    }
    check_dates(payload, "endedAt", now)?;
    let count = player_count(&game["PlaylistId"]);
    let Some(count) = count.filter(|_| game["bHasWinner"] == true && game["bReplay"] == false) else {
        return Err("Match public terminé requis.");
    };
    let teams = check_teams(game)?;
    let winner = teams.iter().find(|team| team_number(&team["TeamNum"]) == winner_team);
    if winner.map(|team| &team["Name"]) != Some(&game["Winner"]) {
        return Err("Gagnant du tableau différent de la fin de match.");
    }
    // After a forfeit the losers often leave at once: the scoreboard keeps whoever is still identified.
    check_players(payload, count)
}

/// A left payload: the last scoreboard of a public match the sender left before its end, with no winner.
fn check_left_match(payload: &Value, now: i64) -> Result<(), &'static str> {
    let state = &payload["state"];
    let game = &state["Game"];
    if payload["left"] != true || payload.get("end").is_some() || payload.get("endedAt").is_some() {
        return Err("Départ du match incohérent.");
    }
    if match_guid(state).is_none() {
        return Err("Match absent ou incohérent.");
    }
    check_dates(payload, "leftAt", now)?;
    let count = player_count(&game["PlaylistId"]);
    let Some(count) = count.filter(|_| game["bReplay"] == false && game.get("bHasWinner").is_none()) else {
        return Err("Match public en cours requis.");
    };
    check_teams(game)?;
    check_players(payload, count)
}

fn match_guid(state: &Value) -> Option<&str> {
    state["MatchGuid"].as_str().filter(|guid| !guid.trim().is_empty() && utf16_len(guid) <= 128)
}

/// `startedAt` and the end or departure date: in order, three hours apart at most, within the last day.
fn check_dates(payload: &Value, key: &str, now: i64) -> Result<(), &'static str> {
    let (Some(started), Some(ended)) = (integer(&payload["startedAt"]), integer(&payload[key])) else {
        return Err("Date du match invalide.");
    };
    if started > ended || ended - started > 10_800_000 || ended > now + 30_000 || ended < now - 86_400_000 {
        return Err("Date du match invalide.");
    }
    Ok(())
}

fn check_teams(game: &Value) -> Result<&[Value], &'static str> {
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
    Ok(teams)
}

/// The scoreboard's identified players and `seenIds`, for a playlist of `count` players.
fn check_players(payload: &Value, count: usize) -> Result<(), &'static str> {
    let players = payload["state"]["Players"].as_array().map(Vec::as_slice).unwrap_or_default();
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

    /// The left payload of the same match: its last scoreboard without a winner, sent when the player left.
    pub fn left_payload(guid: &str) -> Value {
        let ended = payload(guid);
        let mut game = ended["state"]["Game"].clone();
        let game = game.as_object_mut().unwrap();
        game.remove("bHasWinner");
        game.remove("Winner");
        json!({
            "left": true,
            "state": { "MatchGuid": guid, "Players": ended["state"]["Players"], "Game": game },
            "startedAt": NOW - 360_000,
            "leftAt": NOW - 1000,
            "seenIds": ended["seenIds"]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{NOW, left_payload, payload};
    use super::*;
    use serde_json::json;

    fn packet(event: &str, data: Value) -> Value {
        json!({ "Event": event, "Data": data })
    }

    fn dated(mut p: Value) -> Value {
        p["startedAt"] = json!(NOW);
        p["endedAt"] = json!(NOW);
        p["state"] = unhit(p["state"].clone());
        p
    }

    /// The capture counts the crossbar hits of every player it sends: none in these matches.
    fn unhit(mut state: Value) -> Value {
        for player in state["Players"].as_array_mut().unwrap() {
            player["CrossbarHits"] = json!(0);
        }
        state
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
        let player = r#"{"Name":"renamed-player","PrimaryId":"Epic|bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb|0","TeamNum":0,"Score":100,"Goals":0,"Assists":0,"Saves":1,"CrossbarHits":0}"#;
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

        // Without MatchEnded, leaving sends the left payload, never a scoreboard with a winner.
        let mut capture = MatchCapture::default();
        capture.handle(&packet("MatchCreated", guid.clone()), NOW);
        capture.handle(&packet("UpdateState", playing(&p)), NOW);
        let sent = capture.handle(&packet("MatchDestroyed", guid), NOW).unwrap();
        assert_eq!(sent["left"], true);
        assert_eq!(sent.get("end"), None);
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
        // A forfeit: the game may set the clock to 0:00 after the end, the time left before it stays, and so does
        // overtime.
        let sent = clock_sent(json!({ "TimeSeconds": 0, "bOvertime": true }));
        assert_eq!(sent["state"]["Game"]["TimeSeconds"], 143);
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
    fn the_tick_that_reaches_zero_after_the_end_lowers_the_last_second() {
        let p = payload("last-second");
        let mut capture = MatchCapture::default();
        let mut received = Vec::new();
        capture.handle(&packet("MatchCreated", json!({ "MatchGuid": "last-second" })), NOW);
        let mut before = p["state"].clone();
        before["Game"]["TimeSeconds"] = json!(1);
        before["Game"]["bOvertime"] = json!(false);
        capture.handle(&packet("UpdateState", before), NOW);
        capture.handle(&packet("MatchEnded", p["end"].clone()), NOW);
        let mut last = p["state"].clone();
        last["Game"]["TimeSeconds"] = json!(0);
        for _ in 0..2 {
            received.extend(capture.handle(&packet("UpdateState", last.clone()), NOW));
        }
        assert_eq!(received[0]["state"]["Game"]["TimeSeconds"], 0);
    }

    #[test]
    fn sends_the_last_changes_of_the_clock_before_the_end_and_the_first_ones_after() {
        let p = payload("trace");
        let mut capture = MatchCapture::default();
        let mut received = Vec::new();
        capture.handle(&packet("MatchCreated", json!({ "MatchGuid": "trace" })), NOW);
        let update = |time: i64| {
            let mut state = p["state"].clone();
            state["Game"]["TimeSeconds"] = json!(time);
            state["Game"]["bOvertime"] = json!(false);
            packet("UpdateState", state)
        };
        for time in (140..=150).rev() {
            capture.handle(&update(time), NOW);
        }
        capture.handle(&update(140), NOW);
        capture.handle(&packet("MatchEnded", p["end"].clone()), NOW);
        for _ in 0..2 {
            received.extend(capture.handle(&update(0), NOW));
        }
        let mut trace: Vec<Value> =
            (140..=145).rev().map(|time| json!({ "TimeSeconds": time, "bOvertime": false, "ended": false })).collect();
        trace.push(json!({ "TimeSeconds": 0, "bOvertime": false, "ended": true }));
        assert_eq!(received[0]["clockTrace"], Value::Array(trace));
        assert_eq!(received[0]["state"]["Game"]["TimeSeconds"], 140);
        assert_eq!(check_live_match(&received[0], NOW), Ok(()));
    }

    fn without(state: &Value, index: usize) -> Value {
        let mut state = state.clone();
        state["Players"].as_array_mut().unwrap().remove(index);
        state
    }

    /// The scoreboard of a match created, played through `before`, ended, then followed by two identical `after`.
    fn ended_after(guid: &str, before: &[Value], after: &Value) -> Value {
        let p = payload(guid);
        let mut capture = MatchCapture::default();
        let mut received = Vec::new();
        capture.handle(&packet("MatchCreated", json!({ "MatchGuid": guid })), NOW);
        for state in before {
            received.extend(capture.handle(&packet("UpdateState", state.clone()), NOW));
        }
        capture.handle(&packet("MatchEnded", p["end"].clone()), NOW);
        for _ in 0..2 {
            received.extend(capture.handle(&packet("UpdateState", after.clone()), NOW));
        }
        assert_eq!(received.len(), 1);
        received.remove(0)
    }

    #[test]
    fn counts_the_crossbar_hits_of_the_last_toucher_and_sends_the_demolitions() {
        let p = payload("crossbars");
        let mut state = p["state"].clone();
        for (index, player) in state["Players"].as_array_mut().unwrap().iter_mut().enumerate() {
            player["Shortcut"] = json!(index % 3);
            player["Demos"] = json!(index);
        }
        let hit = |shortcut: u64, team: u64| {
            packet(
                "CrossbarHit",
                json!({ "MatchGuid": "crossbars",
                        "BallLastTouch": { "Player": { "Name": "x", "Shortcut": shortcut, "TeamNum": team } } }),
            )
        };
        let mut capture = MatchCapture::default();
        let mut received = Vec::new();
        capture.handle(&packet("MatchCreated", json!({ "MatchGuid": "crossbars" })), NOW);
        capture.handle(&packet("UpdateState", state.clone()), NOW);
        for (shortcut, team) in [(1, 0), (1, 0), (2, 1), (7, 0)] {
            capture.handle(&hit(shortcut, team), NOW);
        }
        capture.handle(&packet("MatchEnded", p["end"].clone()), NOW);
        // After the end, a hit counts for nobody.
        capture.handle(&hit(0, 0), NOW);
        for _ in 0..2 {
            received.extend(capture.handle(&packet("UpdateState", state.clone()), NOW));
        }
        assert_eq!(received.len(), 1);
        let counts: Vec<(Value, Value)> = received[0]["state"]["Players"]
            .as_array()
            .unwrap()
            .iter()
            .map(|player| (player["Demos"].clone(), player["CrossbarHits"].clone()))
            .collect();
        assert_eq!(counts, [(0, 0), (1, 2), (2, 0), (3, 0), (4, 0), (5, 1)].map(|(d, c)| (json!(d), json!(c))));
        // The same JSON as the browser: Demos after Saves, then CrossbarHits, and no Shortcut.
        let first = received[0]["state"]["Players"][1].to_string();
        assert!(first.ends_with(r#""Saves":1,"Demos":1,"CrossbarHits":2}"#), "{first}");
        assert_eq!(check_live_match(&received[0], NOW), Ok(()));
    }

    #[test]
    fn nobody_left_keeps_the_body_byte_for_byte() {
        let p = payload("all-there");
        let sent = ended_after("all-there", &[playing(&p), without(&playing(&p), 2), playing(&p)], &p["state"]);
        assert_eq!(sent.to_string(), dated(p).to_string());
    }

    #[test]
    fn a_club_player_who_left_before_the_end_is_a_departure() {
        let p = payload("left-mid-match");
        let mut before = playing(&p);
        before["Players"][2]["Score"] = json!(150);
        let last = without(&p["state"], 2);
        let sent = ended_after("left-mid-match", &[before, without(&playing(&p), 2)], &last);
        let mut expected = dated(p.clone());
        expected["state"] = unhit(last);
        expected["seenIds"].as_array_mut().unwrap().remove(2);
        let departure = r#"{"PrimaryId":"PS4|222222|0","Name":"Friend Two","TeamNum":0,"Score":150,"Goals":0,"Assists":0,"Saves":1,"CrossbarHits":0}"#;
        let body = expected.to_string();
        assert_eq!(sent.to_string(), format!(r#"{},"departures":[{departure}]}}"#, &body[..body.len() - 1]));
        assert_eq!(check_live_match(&sent, NOW), Ok(()));
    }

    #[test]
    fn a_player_who_came_back_or_left_during_the_podium_is_not_a_departure() {
        let p = payload("came-back");
        let came_back = ended_after("came-back", &[playing(&p), without(&playing(&p), 4), playing(&p)], &p["state"]);
        assert_eq!(came_back.get("departures"), None);
        let podium = ended_after("came-back", &[playing(&p)], &without(&p["state"], 1));
        assert_eq!(podium.get("departures"), None);
        assert_eq!(podium["state"]["Players"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn opponents_gone_before_the_end_are_all_departures() {
        // A 2v2: both opponents quit, the club wins the forfeit, and the scoreboard goes when the match is left.
        let mut p = payload("opponents-gone");
        p["state"]["Game"]["PlaylistId"] = json!(11);
        let players = p["state"]["Players"].as_array_mut().unwrap();
        players.remove(5);
        players.remove(2);
        let guid = json!({ "MatchGuid": "opponents-gone" });
        let end = json!({ "MatchGuid": "opponents-gone", "WinnerTeamNum": 0 });
        let mut capture = MatchCapture::default();
        capture.handle(&packet("MatchCreated", guid.clone()), NOW);
        capture.handle(&packet("UpdateState", playing(&p)), NOW);
        capture.handle(&packet("UpdateState", without(&playing(&p), 3)), NOW);
        capture.handle(&packet("UpdateState", without(&without(&playing(&p), 3), 2)), NOW);
        capture.handle(&packet("MatchEnded", end), NOW);
        let sent = capture.handle(&packet("MatchDestroyed", guid), NOW).unwrap();
        assert_eq!(sent["state"]["Players"].as_array().unwrap().len(), 2);
        let opponents: Vec<Value> = p["state"]["Players"].as_array().unwrap()[2..]
            .iter()
            .map(|player| {
                let mut departure = pick(player, &DEPARTURE_FIELDS);
                departure["CrossbarHits"] = json!(0);
                departure
            })
            .collect();
        assert_eq!(sent["departures"], Value::Array(opponents));
        assert_eq!(sent["departures"][0]["TeamNum"], 1);
        assert_eq!(check_live_match(&sent, NOW), Ok(()));
    }

    #[test]
    fn keeps_eight_departures_at_most_first_seen_first() {
        let p = payload("rotation");
        let own = p["state"]["Players"][0].clone();
        let guest = |index: usize| {
            json!({ "Name": format!("Guest {index}"), "PrimaryId": format!("Steam|{index}|0"), "TeamNum": 1,
                    "Score": 0, "Goals": 0, "Assists": 0, "Saves": 0 })
        };
        let mut before: Vec<Value> = (1..=10)
            .map(|index| {
                let mut state = playing(&p);
                state["Players"] = json!([own, guest(index)]);
                state
            })
            .collect();
        before.push(keep_players(&playing(&p), 1));
        let sent = ended_after("rotation", &before, &keep_players(&p["state"], 1));
        let names: Vec<&str> =
            sent["departures"].as_array().unwrap().iter().map(|d| d["Name"].as_str().unwrap()).collect();
        assert_eq!(names, (1..=8).map(|index| format!("Guest {index}")).collect::<Vec<_>>());
    }

    #[test]
    fn sends_the_left_payload_only_when_the_game_leaves_the_match_before_its_end() {
        let p = payload("left-early");
        let guid = json!({ "MatchGuid": "left-early" });
        let start = |capture: &mut MatchCapture| {
            capture.handle(&packet("MatchCreated", guid.clone()), NOW - 300_000);
            capture.handle(&packet("UpdateState", playing(&p)), NOW - 200_000);
            let mut later = playing(&p);
            later["Players"][0]["Score"] = json!(180);
            later["Game"]["TimeSeconds"] = json!(97);
            capture.handle(&packet("UpdateState", later), NOW - 100_000)
        };
        let mut players = unhit(p["state"].clone())["Players"].clone();
        players[0]["Score"] = json!(180);
        let expected = json!({
            "left": true,
            "state": {
                "MatchGuid": "left-early",
                "Players": players,
                "Game": { "PlaylistId": 13, "Teams": p["state"]["Game"]["Teams"], "bReplay": false }
            },
            "startedAt": NOW - 300_000,
            "leftAt": NOW,
            "seenIds": p["seenIds"]
        });

        let mut capture = MatchCapture::default();
        assert_eq!(start(&mut capture), None);
        let sent = capture.handle(&packet("MatchDestroyed", guid.clone()), NOW).unwrap();
        assert_eq!(sent.to_string(), expected.to_string());
        assert_eq!(check_live_match(&sent, NOW), Ok(()));
        assert!(!capture.in_match());

        // Another match starts: the left payload goes, and the new match is followed.
        for event in ["MatchCreated", "MatchInitialized"] {
            let mut capture = MatchCapture::default();
            start(&mut capture);
            let sent = capture.handle(&packet(event, json!({ "MatchGuid": "next" })), NOW);
            assert_eq!(sent.map(|sent| sent.to_string()), Some(expected.to_string()), "{event}");
            assert!(capture.in_match());
        }

        // The socket closes or the collector stops: nothing tells that the player left, so no departure goes.
        let mut capture = MatchCapture::default();
        start(&mut capture);
        assert_eq!(capture.finish(NOW), None);
        assert!(!capture.in_match());
    }

    #[test]
    fn no_left_payload_for_a_replay_a_private_match_a_match_joined_late_or_one_that_ended() {
        let guid = json!({ "MatchGuid": "not-left" });
        let leave = |prepare: &dyn Fn(&mut MatchCapture), state: Value| {
            let mut capture = MatchCapture::default();
            prepare(&mut capture);
            capture.handle(&packet("UpdateState", state), NOW);
            capture.handle(&packet("MatchDestroyed", guid.clone()), NOW)
        };
        let created = |capture: &mut MatchCapture| {
            capture.handle(&packet("MatchCreated", guid.clone()), NOW);
        };
        let p = payload("not-left");
        let mut replay = playing(&p);
        replay["Game"]["bReplay"] = json!(true);
        assert_eq!(leave(&created, replay), None, "replayed updates");
        let replay_created = |capture: &mut MatchCapture| {
            capture.handle(&packet("ReplayCreated", guid.clone()), NOW);
            capture.handle(&packet("MatchCreated", guid.clone()), NOW);
        };
        assert_eq!(leave(&replay_created, playing(&p)), None, "replay");
        for playlist in [json!(6), json!(40), Value::Null] {
            let mut private = playing(&p);
            private["Game"]["PlaylistId"] = playlist.clone();
            assert_eq!(leave(&created, private), None, "{playlist}");
        }
        assert_eq!(leave(&|_| {}, playing(&p)), None, "joined late");
        assert_eq!(leave(&created, keep_players(&playing(&p), 0)), None, "nobody identified");

        // A match that ended normally sent its scoreboard: leaving it sends nothing more.
        let mut capture = MatchCapture::default();
        let mut received = Vec::new();
        created(&mut capture);
        received.extend(capture.handle(&packet("UpdateState", playing(&p)), NOW));
        finish(&mut capture, &p, &mut received);
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].get("left"), None);
        assert_eq!(capture.handle(&packet("MatchDestroyed", guid.clone()), NOW), None);
        assert_eq!(capture.finish(NOW), None);
    }

    #[test]
    fn validation_accepts_the_left_payload_and_refuses_a_broken_one() {
        assert_eq!(check_live_match(&left_payload("left"), NOW), Ok(()));
        let mutations: Vec<fn(&mut Value)> = vec![
            |p| p["left"] = json!(false),
            |p| p["left"] = json!("true"),
            |p| p["end"] = json!({ "MatchGuid": "left", "WinnerTeamNum": 1 }),
            |p| p["endedAt"] = json!(NOW),
            |p| _ = p.as_object_mut().unwrap().remove("leftAt"),
            |p| p["leftAt"] = json!(NOW - 400_000),
            |p| p["leftAt"] = json!(NOW + 60_000),
            |p| p["leftAt"] = json!(NOW - 86_400_001),
            |p| p["state"]["MatchGuid"] = json!(" "),
            |p| p["state"]["Game"]["bReplay"] = json!(true),
            |p| _ = p["state"]["Game"].as_object_mut().unwrap().remove("bReplay"),
            |p| p["state"]["Game"]["bHasWinner"] = json!(true),
            |p| p["state"]["Game"]["PlaylistId"] = json!(6),
            |p| _ = p["state"]["Game"]["Teams"].as_array_mut().unwrap().pop(),
            |p| {
                p["state"]["Players"] = json!([]);
                p["seenIds"] = json!([]);
            },
            |p| _ = p["seenIds"].as_array_mut().unwrap().pop(),
            |p| p["state"]["Players"][0]["PrimaryId"] = json!("Bot|1|0"),
            |p| p["state"]["Players"][3]["TeamNum"] = json!(0),
            |p| p["state"]["Players"][0]["Name"] = json!(""),
        ];
        for (index, mutate) in mutations.iter().enumerate() {
            let mut p = left_payload("left");
            mutate(&mut p);
            assert!(check_live_match(&p, NOW).is_err(), "mutation {index}");
        }
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
