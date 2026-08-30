use serde::Serialize;

pub mod news;
pub mod updater;

#[derive(Debug, Serialize)]
pub struct LauncherError {
    pub code: &'static str,
    pub message: &'static str,
    pub detail: String,
    pub retryable: bool,
}

impl From<String> for LauncherError {
    fn from(detail: String) -> Self {
        let lower = detail.to_lowercase();
        let (code, message, retryable) = if lower.contains("os error 112")
            || lower.contains("espace disque")
            || lower.contains("not enough space")
        {
            (
                "disk_full",
                "L’espace disque disponible est insuffisant.",
                false,
            )
        } else if lower.contains("os error 32")
            || lower.contains("utilisé par un autre processus")
            || lower.contains("being used by another process")
        {
            (
                "client_running",
                "Ferme le jeu, puis relance la mise à jour.",
                true,
            )
        } else if lower.contains("os error 5") || lower.contains("permission denied") {
            (
                "permission",
                "Le launcher ne peut pas écrire dans ce dossier.",
                false,
            )
        } else if lower.contains("mise à jour obligatoire") {
            (
                "update_required",
                "Une nouvelle mise à jour est obligatoire avant de jouer.",
                true,
            )
        } else if lower.contains("connexion")
            || lower.contains("lecture réseau")
            || lower.contains("manifeste indisponible")
            || lower.contains("téléchargement du manifeste")
            || lower.contains("initialisation réseau")
        {
            (
                "network",
                "Connexion au serveur de mise à jour interrompue.",
                true,
            )
        } else if lower.contains("sha-256 incorrect") {
            (
                "corrupt_download",
                "Le fichier téléchargé est corrompu et doit être repris.",
                true,
            )
        } else if lower.contains("signature")
            || lower.contains("séquence")
            || lower.contains("manifeste json invalide")
            || lower.contains("url https officielle")
        {
            (
                "security",
                "La mise à jour n’a pas pu être authentifiée.",
                false,
            )
        } else {
            ("unknown", "Le launcher a rencontré une erreur.", false)
        };
        Self {
            code,
            message,
            detail,
            retryable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_errors_expose_a_safe_action_instead_of_raw_network_text() {
        let network = LauncherError::from(
            "Lecture réseau interrompue : request or response body error".to_string(),
        );
        assert_eq!(network.code, "network");
        assert!(network.retryable);

        let security = LauncherError::from("Signature du manifeste invalide.".to_string());
        assert_eq!(security.code, "security");
        assert!(!security.retryable);

        let update = LauncherError::from("Mise à jour obligatoire avant de jouer.".to_string());
        assert_eq!(update.code, "update_required");
        assert!(update.retryable);
    }
}
