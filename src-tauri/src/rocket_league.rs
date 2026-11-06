//! Finds Rocket League on this PC and turns on its Stats API, so the player has nothing to edit by hand.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The same values as the instructions on the /direct page of the web app.
const SECTION: &str = "TAGame.MatchStatsExporter_TA";
const SETTINGS: [(&str, &str); 3] = [("PacketSendRate", "10"), ("Port", "0"), ("WebPort", "49124")];

/// Rocket League reads TAStatsAPI.ini when it exists, DefaultStatsAPI.ini otherwise.
const CONFIG_FILES: [&str; 2] = ["TAStatsAPI.ini", "DefaultStatsAPI.ini"];

const STEAM_APP_ID: &str = "252950";
const EPIC_APP_NAME: &str = "Sugar";

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Updated(PathBuf),
    AlreadySet,
    NoConfigFile,
}

#[derive(Debug)]
pub struct SetupError(io::Error);

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.kind() {
            io::ErrorKind::PermissionDenied => write!(f, "accès refusé au dossier du jeu"),
            io::ErrorKind::InvalidData => write!(f, "fichier de réglages dans un format inattendu"),
            _ => write!(f, "lecture ou écriture impossible ({})", self.0),
        }
    }
}

impl From<io::Error> for SetupError {
    fn from(error: io::Error) -> Self {
        Self(error)
    }
}

/// Every Rocket League folder found through the Epic and Steam launchers, without duplicates.
pub fn installs() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for install in epic_installs().into_iter().chain(steam_installs()) {
        if install.join("TAGame").is_dir() && !found.contains(&install) {
            found.push(install);
        }
    }
    found
}

/// Sets the Stats API keys, keeping every other line, after a one-time backup of the original file.
pub fn configure(install: &Path) -> Result<Outcome, SetupError> {
    let folder = install.join("TAGame").join("Config");
    let Some(file) = CONFIG_FILES
        .iter()
        .map(|name| folder.join(name))
        .find(|file| file.is_file())
    else {
        return Ok(Outcome::NoConfigFile);
    };
    let bytes = fs::read(&file)?;
    let text = String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let patched = with_stats_api(&text);
    if patched == text {
        return Ok(Outcome::AlreadySet);
    }
    let backup = file.with_extension("ini.bak");
    if !backup.exists() {
        fs::copy(&file, &backup)?;
    }
    let draft = file.with_extension("ini.tmp");
    fs::write(&draft, patched)?;
    fs::rename(&draft, &file)?;
    Ok(Outcome::Updated(file))
}

/// Returns the ini text with the Stats API keys set, leaving comments, other keys and line endings alone.
fn with_stats_api(text: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let ends_with_newline = body.ends_with('\n');
    let mut lines: Vec<String> = body.lines().map(str::to_owned).collect();

    let Some(header) = lines.iter().position(|line| is_header(line, Some(SECTION))) else {
        while lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.pop();
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("[{SECTION}]"));
        lines.extend(SETTINGS.iter().map(|(key, value)| format!("{key}={value}")));
        return format!("{bom}{}{newline}", lines.join(newline));
    };

    let end = lines[header + 1..]
        .iter()
        .position(|line| is_header(line, None))
        .map_or(lines.len(), |offset| header + 1 + offset);
    let mut missing: Vec<&(&str, &str)> = SETTINGS.iter().collect();
    for line in &mut lines[header + 1..end] {
        let Some((key, _)) = line.split_once('=') else { continue };
        let key = key.trim().to_owned();
        if let Some((name, value)) = SETTINGS.iter().find(|(name, _)| name.eq_ignore_ascii_case(&key)) {
            *line = format!("{key}={value}");
            missing.retain(|(other, _)| other != name);
        }
    }
    let last_setting = lines[header..end]
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(header, |offset| header + offset);
    let added: Vec<String> = missing.iter().map(|(key, value)| format!("{key}={value}")).collect();
    lines.splice(last_setting + 1..last_setting + 1, added);

    let joined = lines.join(newline);
    if ends_with_newline {
        format!("{bom}{joined}{newline}")
    } else {
        format!("{bom}{joined}")
    }
}

fn is_header(line: &str, name: Option<&str>) -> bool {
    let line = line.trim();
    match line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
        Some(found) => name.is_none_or(|name| found.trim().eq_ignore_ascii_case(name)),
        None => false,
    }
}

fn epic_installs() -> Vec<PathBuf> {
    let Some(program_data) = std::env::var_os("ProgramData") else {
        return Vec::new();
    };
    let manifests = PathBuf::from(program_data).join(r"Epic\EpicGamesLauncher\Data\Manifests");
    let Ok(entries) = fs::read_dir(manifests) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("item"))
        })
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|manifest| epic_install(&manifest))
        .collect()
}

fn epic_install(manifest: &str) -> Option<PathBuf> {
    let manifest: serde_json::Value = serde_json::from_str(manifest).ok()?;
    let text = |key: &str| manifest.get(key).and_then(serde_json::Value::as_str);
    let is_rocket_league = text("AppName") == Some(EPIC_APP_NAME)
        || text("DisplayName").is_some_and(|name| name.starts_with("Rocket League"));
    if !is_rocket_league {
        return None;
    }
    text("InstallLocation")
        .filter(|location| !location.is_empty())
        .map(PathBuf::from)
}

fn steam_installs() -> Vec<PathBuf> {
    let Some(steam) = steam_folder() else { return Vec::new() };
    let mut libraries = vec![steam.clone()];
    if let Ok(folders) = fs::read_to_string(steam.join(r"steamapps\libraryfolders.vdf")) {
        libraries.extend(vdf_values(&folders, "path").into_iter().map(PathBuf::from));
    }
    libraries
        .into_iter()
        .filter_map(|library| {
            let apps = library.join("steamapps");
            let manifest = fs::read_to_string(apps.join(format!("appmanifest_{STEAM_APP_ID}.acf"))).ok()?;
            let folder = vdf_values(&manifest, "installdir").into_iter().next()?;
            Some(apps.join("common").join(folder))
        })
        .collect()
}

#[cfg(windows)]
fn steam_folder() -> Option<PathBuf> {
    let user = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let path: String = user
        .open_subkey(r"Software\Valve\Steam")
        .ok()?
        .get_value("SteamPath")
        .ok()?;
    Some(PathBuf::from(path.replace('/', r"\")))
}

#[cfg(not(windows))]
fn steam_folder() -> Option<PathBuf> {
    None
}

/// The values of a key in Valve's text format: `"key"  "value"`, with backslashes escaped.
fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let quoted = format!("\"{key}\"");
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix(&quoted)?.trim();
            let value = rest.strip_prefix('"')?.strip_suffix('"')?;
            Some(value.replace(r"\\", r"\"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_the_values_and_keeps_the_rest() {
        let before = "; Stats API\r\n[TAGame.MatchStatsExporter_TA]\r\nPort=49123\r\nPacketSendRate=0\r\nOther=1\r\n\r\n[Next]\r\nPort=7\r\n";
        let after = "; Stats API\r\n[TAGame.MatchStatsExporter_TA]\r\nPort=0\r\nPacketSendRate=10\r\nOther=1\r\nWebPort=49124\r\n\r\n[Next]\r\nPort=7\r\n";
        assert_eq!(with_stats_api(before), after);
    }

    #[test]
    fn leaves_a_ready_file_untouched() {
        let ready = "[TAGame.MatchStatsExporter_TA]\nPacketSendRate=10\nPort=0\nWebPort=49124\n";
        assert_eq!(with_stats_api(ready), ready);
    }

    #[test]
    fn adds_the_section_when_missing() {
        assert_eq!(
            with_stats_api("\u{feff}[Core]\nA=1\n"),
            "\u{feff}[Core]\nA=1\n\n[TAGame.MatchStatsExporter_TA]\nPacketSendRate=10\nPort=0\nWebPort=49124\n"
        );
    }

    #[test]
    fn matches_keys_and_section_regardless_of_case_and_spaces() {
        assert_eq!(
            with_stats_api("[tagame.matchstatsexporter_ta]\nport = 49123\n"),
            "[tagame.matchstatsexporter_ta]\nport=0\nPacketSendRate=10\nWebPort=49124\n"
        );
    }

    #[test]
    fn reads_steam_libraries() {
        let folders = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\Games\"\n\t}\n}\n";
        assert_eq!(
            vdf_values(folders, "path"),
            [r"C:\Program Files (x86)\Steam", r"D:\Games"]
        );
    }

    #[test]
    fn reads_the_epic_manifest_of_rocket_league_only() {
        let rocket_league =
            r#"{"AppName":"Sugar","DisplayName":"Rocket League®","InstallLocation":"D:\\Epic\\rocketleague"}"#;
        let other = r#"{"AppName":"Fortnite","DisplayName":"Fortnite","InstallLocation":"D:\\Epic\\Fortnite"}"#;
        assert_eq!(
            epic_install(rocket_league),
            Some(PathBuf::from(r"D:\Epic\rocketleague"))
        );
        assert_eq!(epic_install(other), None);
    }

    #[test]
    fn configures_once_with_a_backup() {
        let install = std::env::temp_dir().join(format!("brc-direct-test-{}", std::process::id()));
        let folder = install.join("TAGame").join("Config");
        fs::create_dir_all(&folder).unwrap();
        let file = folder.join("DefaultStatsAPI.ini");
        let original = "[TAGame.MatchStatsExporter_TA]\nPort=49123\n";
        fs::write(&file, original).unwrap();

        assert_eq!(configure(&install).unwrap(), Outcome::Updated(file.clone()));
        assert_eq!(
            fs::read_to_string(folder.join("DefaultStatsAPI.ini.bak")).unwrap(),
            original
        );
        assert_eq!(configure(&install).unwrap(), Outcome::AlreadySet);

        fs::remove_dir_all(&install).unwrap();
        assert_eq!(configure(&install).unwrap(), Outcome::NoConfigFile);
    }
}
