//! The background threads (game, uploads, updates) and the state they share with the window.

use crate::api::{Api, Pairing};
use crate::bridge::{Event, Next, Queue};
use crate::game;
use crate::status::{Status, Update};
use crate::store::Settings;
use crate::update::{Release, Updater};
use serde_json::Value;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

const RETRY: Duration = Duration::from_secs(10);
const UPDATE_CHECK: Duration = Duration::from_secs(6 * 60 * 60);
const UPDATE_CHECK_AFTER_FAILURE: Duration = Duration::from_secs(60 * 60);

/// Where the token lives between launches; on Windows, encrypted for this user with DPAPI.
pub trait Vault: Send + Sync {
    fn save(&self, token: &str) -> io::Result<()>;
    fn clear(&self);
}

enum Command {
    Match(Value),
    Wake,
}

pub struct Service {
    status: Mutex<Status>,
    token: Mutex<Option<String>>,
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    vault: Box<dyn Vault>,
    api: Api,
    notify: Box<dyn Fn() + Send + Sync>,
    commands: Mutex<Option<Sender<Command>>>,
    updater: Updater,
    release: Mutex<Option<Release>>,
    toasts: AtomicU32,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Service {
    pub fn new(
        status: Status,
        token: Option<String>,
        settings_path: PathBuf,
        vault: Box<dyn Vault>,
        notify: Box<dyn Fn() + Send + Sync>,
    ) -> Arc<Self> {
        let settings = Settings::load(&settings_path);
        Arc::new(Self {
            status: Mutex::new(status),
            token: Mutex::new(token),
            settings: Mutex::new(settings),
            settings_path,
            vault,
            api: Api::new(),
            notify,
            commands: Mutex::new(None),
            updater: Updater::new(),
            release: Mutex::new(None),
            toasts: AtomicU32::new(0),
        })
    }

    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    pub fn change_settings(&self, change: impl FnOnce(&mut Settings)) {
        let mut settings = lock(&self.settings);
        change(&mut settings);
        settings.save(&self.settings_path);
    }

    /// Changes the status, and tells the window only when something it shows changed.
    fn update(&self, change: impl FnOnce(&mut Status)) {
        let changed = {
            let mut status = lock(&self.status);
            let before = status.clone();
            change(&mut status);
            *status != before
        };
        if changed {
            (self.notify)();
        }
    }

    /// Starts listening to the game and sending matches, and checks the saved token once.
    pub fn start(self: &Arc<Self>, queue_path: PathBuf) {
        let (sender, receiver) = mpsc::channel();
        // The first pass sends what an earlier run left in the queue.
        let _ = sender.send(Command::Wake);
        *lock(&self.commands) = Some(sender.clone());

        let service = Arc::clone(self);
        thread::spawn(move || {
            game::listen(
                |connected| {
                    service.update(|status| status.game_connected = connected);
                    if !connected {
                        service.match_over();
                    }
                },
                |payload| {
                    let _ = sender.send(Command::Match(payload));
                },
                |playing| {
                    if playing {
                        service.update(|status| status.in_match = true);
                    } else {
                        service.match_over();
                    }
                },
            );
        });

        let service = Arc::clone(self);
        thread::spawn(move || service.send_matches(Queue::load(queue_path, game::now()), receiver));

        let service = Arc::clone(self);
        thread::spawn(move || {
            loop {
                let reached = service.check_for_update(false);
                thread::sleep(if reached { UPDATE_CHECK } else { UPDATE_CHECK_AFTER_FAILURE });
            }
        });

        if let Some(token) = lock(&self.token).clone() {
            let service = Arc::clone(self);
            thread::spawn(move || match service.api.player(&token) {
                Some(Some(player)) => {
                    service.change_settings(|settings| settings.player = Some(player.clone()));
                    service.update(|status| status.player = Some(player));
                }
                Some(None) => service.forget_token(),
                None => {}
            });
        }
    }

    fn wake(&self) {
        if let Some(sender) = lock(&self.commands).as_ref() {
            let _ = sender.send(Command::Wake);
        }
    }

    /// Owns the queue: waits for a match or a retry, without any work in between.
    fn send_matches(&self, mut queue: Queue, commands: Receiver<Command>) {
        let mut retry_later = false;
        loop {
            self.update(|status| status.queued = queue.len());
            let command = if retry_later {
                match commands.recv_timeout(RETRY) {
                    Err(RecvTimeoutError::Timeout) => Command::Wake,
                    Err(RecvTimeoutError::Disconnected) => return,
                    Ok(command) => command,
                }
            } else {
                match commands.recv() {
                    Ok(command) => command,
                    Err(_) => return,
                }
            };
            if let Command::Match(payload) = command
                && !queue.add(payload)
            {
                self.update(|status| status.sending_problem = Some("Plus de 20 matchs en attente.".into()));
            }
            let Some(token) = lock(&self.token).clone() else {
                retry_later = false;
                continue;
            };
            let next = queue.upload(game::now(), |body| self.api.send_match(&token, body), |event| self.report(event));
            if queue.len() == 0 {
                self.update(|status| status.sending_problem = None);
            }
            retry_later = next == Next::Retry;
        }
    }

    fn report(&self, event: Event) {
        match event {
            Event::Received => self.update(|status| {
                status.last_received = true;
                status.sending_problem = None;
            }),
            Event::Offline => self.update(|status| status.sending_problem = Some("Pas de connexion au site.".into())),
            Event::Unconfirmed => {
                self.update(|status| status.sending_problem = Some("Réception non confirmée par le site.".into()))
            }
            Event::Failed(error) => self.update(|status| status.sending_problem = Some(error)),
            Event::Dropped(_) => {}
            Event::Revoked => self.forget_token(),
        }
    }

    fn forget_token(&self) {
        *lock(&self.token) = None;
        self.vault.clear();
        self.update(|status| {
            status.needs_pairing = true;
            status.pairing_error = Some("L’association a été retirée : entre un nouveau code.".into());
            status.sending_problem = None;
        });
    }

    fn toast(&self, text: String) {
        let id = self.toasts.fetch_add(1, Ordering::Relaxed) + 1;
        self.update(|status| status.toast = Some((id, text)));
    }

    /// Asks GitHub for a newer release; true when GitHub answered.
    fn check_for_update(&self, manual: bool) -> bool {
        if manual {
            self.update(|status| {
                if !status.update.busy() {
                    status.update = Update::Checking;
                }
            });
        }
        let answer = self.updater.check();
        if let Some(Some(release)) = &answer {
            *lock(&self.release) = Some(release.clone());
        }
        self.update(|status| {
            if status.update.busy() {
                return;
            }
            status.update = match (&answer, &status.update) {
                (Some(Some(release)), _) => Update::Available(release.version.clone()),
                (Some(None), _) if manual => Update::UpToDate,
                (None, _) if manual => Update::CheckFailed,
                (_, current) => current.clone(),
            };
        });
        answer.is_some()
    }

    /// « Rechercher les mises à jour »: answers in the window.
    pub fn check_now(self: &Arc<Self>) {
        let service = Arc::clone(self);
        thread::spawn(move || service.check_for_update(true));
    }

    /// « Mettre à jour »: now, or at the end of the match being played.
    pub fn request_update(self: &Arc<Self>) {
        let status = self.status();
        let Some(version) = status.update.offer().map(str::to_owned) else { return };
        if status.in_match {
            self.update(|status| status.update = Update::AfterMatch(version));
            self.toast("Mise à jour prévue à la fin du match.".into());
        } else {
            self.start_update();
        }
    }

    /// The match ended, or the game closed: a waiting update can run.
    fn match_over(self: &Arc<Self>) {
        self.update(|status| status.in_match = false);
        if matches!(self.status().update, Update::AfterMatch(_)) {
            self.start_update();
        }
    }

    fn start_update(self: &Arc<Self>) {
        let Some(release) = lock(&self.release).clone() else { return };
        self.update(|status| status.update = Update::Downloading(release.version.clone()));
        self.toast(format!("Mise à jour vers la version {} en cours.", release.version));
        let service = Arc::clone(self);
        thread::spawn(move || {
            let launched = service.updater.download(&release).ok().is_some_and(|installer| launch(&installer));
            // Installing makes the window quit, so that the installer can replace the executable.
            service.update(|status| {
                status.update = if launched { Update::Installing } else { Update::Failed(release.version.clone()) }
            });
        });
    }

    /// A typed code that cannot be right: told at once, without asking the API.
    pub fn wrong_code(&self) {
        self.update(|status| status.pairing_error = Some("Le code fait 8 lettres ou chiffres.".into()));
    }

    /// Trades a code for a token in the background; the window follows through the status.
    pub fn pair(self: &Arc<Self>, code: String) {
        self.update(|status| {
            status.pairing = true;
            status.pairing_error = None;
        });
        let service = Arc::clone(self);
        thread::spawn(move || {
            let error = match service.api.pair(&code) {
                Pairing::Paired { token, player } => match service.vault.save(&token) {
                    Ok(()) => {
                        *lock(&service.token) = Some(token);
                        service.change_settings(|settings| settings.player = Some(player.clone()));
                        service.update(|status| {
                            status.needs_pairing = false;
                            status.pairing = false;
                            status.player = Some(player);
                        });
                        service.wake();
                        return;
                    }
                    Err(_) => "Impossible d’enregistrer l’association sur ce PC.".to_owned(),
                },
                Pairing::Refused(error) => error,
                Pairing::Offline => "Site injoignable : vérifie la connexion Internet.".to_owned(),
            };
            service.update(|status| {
                status.pairing = false;
                status.pairing_error = Some(error);
            });
        });
    }
}

/// Runs the per-user installer quietly; it restarts the collector when it is done.
fn launch(installer: &std::path::Path) -> bool {
    let quiet = std::process::Command::new(installer).args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]).spawn();
    quiet.or_else(|_| std::process::Command::new(installer).spawn()).is_ok()
}

/// Codes are typed by hand: spaces, dashes and case do not matter.
pub fn clean_code(typed: &str) -> Option<String> {
    let code: String = typed.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_uppercase();
    (code.len() == 8).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::clean_code;

    #[test]
    fn codes_ignore_spaces_dashes_and_case() {
        assert_eq!(clean_code(" ab12-cd34 ").as_deref(), Some("AB12CD34"));
        assert_eq!(clean_code("AB12CD3"), None);
    }
}
