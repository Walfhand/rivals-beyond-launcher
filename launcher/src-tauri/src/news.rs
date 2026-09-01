use crate::updater::verify_signed_document;
use reqwest::{blocking::Client, header::ACCEPT_ENCODING, Url};
use serde::{Deserialize, Serialize};
use std::{io::Read, time::Duration};

const MAX_NEWS_SIZE: u64 = 512 * 1024;
const NEWS_TIMEOUT: Duration = Duration::from_secs(30);
const OFFICIAL_HOST: &str = "moba-data.nbg1.your-objectstorage.com";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NewsHero {
    pub eyebrow: String,
    pub title: String,
    pub summary: String,
    pub cta: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NewsItem {
    pub slug: String,
    pub category: String,
    pub title: String,
    pub summary: String,
    pub published_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NewsFeed {
    pub schema_version: u32,
    pub updated_at: String,
    pub hero: NewsHero,
    pub items: Vec<NewsItem>,
}

impl NewsFeed {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || !valid_date(&self.updated_at) {
            return Err("Version ou date du flux de nouveautés invalide.".into());
        }
        if self.items.is_empty() || self.items.len() > 6 {
            return Err("Nombre de nouveautés invalide.".into());
        }
        for value in [
            &self.hero.eyebrow,
            &self.hero.title,
            &self.hero.summary,
            &self.hero.cta,
        ] {
            validate_text(value, 240)?;
        }
        for item in &self.items {
            if !valid_slug(&item.slug) {
                return Err("Slug de nouveauté invalide.".into());
            }
            validate_text(&item.category, 40)?;
            validate_text(&item.title, 100)?;
            validate_text(&item.summary, 320)?;
            if !valid_date(&item.published_at) {
                return Err("Date de nouveauté invalide.".into());
            }
        }
        Ok(())
    }
}

pub fn fetch_news(
    client: &Client,
    news_url: &str,
    public_key: [u8; 32],
) -> Result<NewsFeed, String> {
    validate_news_url(news_url)?;
    let mut response = client
        .get(news_url)
        .header(ACCEPT_ENCODING, "identity")
        .timeout(NEWS_TIMEOUT)
        .send()
        .map_err(|error| format!("Téléchargement des nouveautés impossible : {error}"))?;
    if !response.status().is_success() {
        return Err(format!("Nouveautés indisponibles ({}).", response.status()));
    }
    if response.content_length().unwrap_or(0) > MAX_NEWS_SIZE {
        return Err("Le flux de nouveautés est trop volumineux.".into());
    }
    let mut document = Vec::new();
    response
        .by_ref()
        .take(MAX_NEWS_SIZE + 1)
        .read_to_end(&mut document)
        .map_err(|error| format!("Lecture des nouveautés impossible : {error}"))?;
    load_signed_news(&document, public_key)
}

pub fn load_signed_news(document: &[u8], public_key: [u8; 32]) -> Result<NewsFeed, String> {
    if document.len() as u64 > MAX_NEWS_SIZE {
        return Err("Le flux de nouveautés signé est trop volumineux.".into());
    }
    let payload = verify_signed_document(document, public_key)?;
    let feed: NewsFeed = serde_json::from_str(&payload)
        .map_err(|error| format!("Flux de nouveautés JSON invalide : {error}"))?;
    feed.validate()?;
    Ok(feed)
}

fn validate_news_url(value: &str) -> Result<(), String> {
    let url =
        Url::parse(value).map_err(|error| format!("URL des nouveautés invalide : {error}"))?;
    if url.scheme() != "https"
        || url.host_str() != Some(OFFICIAL_HOST)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Le flux doit utiliser l’URL HTTPS officielle.".into());
    }
    Ok(())
}

fn validate_text(value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() || value.chars().count() > max || value.chars().any(char::is_control) {
        return Err("Texte de nouveauté invalide.".into());
    }
    Ok(())
}

fn valid_date(value: &str) -> bool {
    value.len() == 10
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7) {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn signed(feed: &NewsFeed) -> (Vec<u8>, [u8; 32]) {
        let payload = serde_json::to_string(feed).unwrap();
        let signing = SigningKey::from_bytes(&[9; 32]);
        let signature = signing.sign(payload.as_bytes());
        let document = serde_json::json!({
            "schema_version": 1,
            "payload": payload,
            "signature": signature.to_bytes().iter().map(|byte| format!("{byte:02x}")).collect::<String>()
        });
        (
            serde_json::to_vec(&document).unwrap(),
            signing.verifying_key().to_bytes(),
        )
    }

    #[test]
    fn signed_news_accepts_text_but_rejects_tampering() {
        let feed = NewsFeed {
            schema_version: 1,
            updated_at: "2026-08-28".into(),
            hero: NewsHero {
                eyebrow: "SAISON 0".into(),
                title: "L’appel du Nexus".into(),
                summary: "Découvrez The Last Divide.".into(),
                cta: "Voir les nouveautés".into(),
            },
            items: vec![NewsItem {
                slug: "le-sanctuaire".into(),
                category: "CARTE".into(),
                title: "Le Sanctuaire".into(),
                summary: "Le nouveau lobby.".into(),
                published_at: "2026-08-28".into(),
            }],
        };
        let (document, key) = signed(&feed);
        assert_eq!(load_signed_news(&document, key).unwrap().items.len(), 1);

        let mut tampered = document;
        let position = tampered
            .windows(6)
            .position(|part| part == b"SAISON")
            .unwrap();
        tampered[position] = b'X';
        assert!(load_signed_news(&tampered, key)
            .unwrap_err()
            .contains("Signature"));

        let mut unsafe_feed = feed;
        unsafe_feed.items[0].slug = "../register".into();
        let (document, key) = signed(&unsafe_feed);
        assert!(load_signed_news(&document, key)
            .unwrap_err()
            .contains("Slug"));
    }

    #[test]
    fn tracked_news_source_matches_the_runtime_schema() {
        let feed: NewsFeed = serde_json::from_str(include_str!("../../news.json")).unwrap();
        feed.validate().unwrap();
        assert_eq!(feed.items.len(), 3);
    }
}
