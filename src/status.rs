//! What the collector knows, and the one line the player sees: green, grey or red.

/// The result of setting up Rocket League's Stats API at launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setup {
    Ready,
    /// The ini file was just changed: the game must be restarted if it is running.
    Changed,
    Problem(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub setup: Setup,
    pub game_connected: bool,
    pub player: Option<String>,
    pub needs_pairing: bool,
    pub pairing: bool,
    pub pairing_error: Option<String>,
    pub queued: usize,
    pub sending_problem: Option<String>,
    pub last_received: bool,
}

impl Status {
    pub fn new(setup: Setup, player: Option<String>, paired: bool) -> Self {
        Self {
            setup,
            game_connected: false,
            player,
            needs_pairing: !paired,
            pairing: false,
            pairing_error: None,
            queued: 0,
            sending_problem: None,
            last_received: false,
        }
    }

    /// Paired and Rocket League already set: the collector can start out of sight.
    pub fn starts_hidden(&self) -> bool {
        !self.needs_pairing && self.setup == Setup::Ready
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    Live,
    Waiting,
    Problem,
}

#[derive(Debug, PartialEq, Eq)]
pub struct View {
    pub light: Light,
    pub title: String,
    pub detail: String,
}

fn matches(count: usize) -> String {
    if count > 1 { format!("{count} matchs en attente") } else { format!("{count} match en attente") }
}

pub fn view(status: &Status) -> View {
    let (light, title, detail) = if status.needs_pairing {
        let detail = match (&status.pairing_error, status.pairing) {
            (_, true) => "Association en cours…".to_owned(),
            (Some(error), false) => error.clone(),
            (None, false) => "Entre le code affiché dans ton profil Big Rocket Camp.".to_owned(),
        };
        (Light::Problem, "Association requise", detail)
    } else if let Some(problem) = &status.sending_problem {
        (Light::Problem, "Envoi en attente", format!("{problem} {}.", matches(status.queued)))
    } else if status.game_connected {
        let detail = if status.last_received {
            "Dernier match bien reçu par le club."
        } else {
            "Les matchs partent tout seuls à la fin."
        };
        (Light::Live, "En direct", detail.to_owned())
    } else if let Setup::Problem(problem) = &status.setup {
        (Light::Problem, "Rocket League à régler", format!("{problem} Voir l’aide."))
    } else if status.setup == Setup::Changed {
        (Light::Waiting, "Rocket League réglé", "Relance le jeu s’il est ouvert.".to_owned())
    } else {
        (Light::Waiting, "En attente de Rocket League", "Lance le jeu : la pastille passera au vert.".to_owned())
    };
    View { light, title: title.to_owned(), detail }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paired() -> Status {
        Status::new(Setup::Ready, Some("player-one".into()), true)
    }

    #[test]
    fn green_only_while_the_game_is_connected() {
        let mut status = paired();
        assert_eq!(view(&status).light, Light::Waiting);
        status.game_connected = true;
        assert_eq!(view(&status).light, Light::Live);
    }

    #[test]
    fn red_for_pairing_sending_or_setup_problems() {
        let mut status = Status::new(Setup::Ready, None, false);
        assert_eq!(view(&status).title, "Association requise");
        status = paired();
        status.game_connected = true;
        status.sending_problem = Some("Pas de connexion Internet.".into());
        status.queued = 2;
        assert_eq!(view(&status).detail, "Pas de connexion Internet. 2 matchs en attente.");
        status = paired();
        status.setup = Setup::Problem("Dossier du jeu protégé.".into());
        assert_eq!(view(&status).light, Light::Problem);
        status.game_connected = true;
        assert_eq!(view(&status).light, Light::Live);
    }

    #[test]
    fn hides_at_launch_only_once_paired_and_set() {
        assert!(paired().starts_hidden());
        assert!(!Status::new(Setup::Changed, Some("player-one".into()), true).starts_hidden());
        assert!(!Status::new(Setup::Ready, None, false).starts_hidden());
    }
}
