use reqwest::{blocking::Client, header::ACCEPT_ENCODING, Url};
use serde::{Deserialize, Serialize};
use std::{io::Read, time::Duration};

const MAX_NEWS_SIZE: u64 = 512 * 1024;
const NEWS_TIMEOUT: Duration = Duration::from_secs(30);
const NEWS_API_URL: &str = "https://api.rivalsbeyond.com/api/v1/news";
const NEWS_COUNT: usize = 3;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsItem {
    pub slug: String,
    pub locale: String,
    pub title: String,
    pub summary: String,
    pub published_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NewsFeed {
    pub items: Vec<NewsItem>,
}

fn web_locale(locale: &str) -> Result<&'static str, String> {
    match locale {
        "frFR" => Ok("fr"),
        "enUS" => Ok("en"),
        _ => Err("Unsupported news language.".into()),
    }
}

fn news_url(locale: &str) -> Result<Url, String> {
    let mut url = Url::parse(NEWS_API_URL).map_err(|error| error.to_string())?;
    url.query_pairs_mut().extend_pairs([
        ("locale", web_locale(locale)?),
        ("page", "1"),
        ("pageSize", "3"),
    ]);
    Ok(url)
}

pub fn fetch_news(client: &Client, locale: &str) -> Result<NewsFeed, String> {
    let mut response = client
        .get(news_url(locale)?)
        .header(ACCEPT_ENCODING, "identity")
        .timeout(NEWS_TIMEOUT)
        .send()
        .map_err(|error| format!("Cannot fetch news: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("News unavailable ({}).", response.status()));
    }
    if response.content_length().unwrap_or(0) > MAX_NEWS_SIZE {
        return Err("News response is too large.".into());
    }
    let mut document = Vec::new();
    response
        .by_ref()
        .take(MAX_NEWS_SIZE + 1)
        .read_to_end(&mut document)
        .map_err(|error| format!("Cannot read news: {error}"))?;
    load_news(&document, locale)
}

fn load_news(document: &[u8], locale: &str) -> Result<NewsFeed, String> {
    if document.len() as u64 > MAX_NEWS_SIZE {
        return Err("News response is too large.".into());
    }
    let expected = web_locale(locale)?;
    let feed: NewsFeed = serde_json::from_slice(document)
        .map_err(|error| format!("Invalid news response: {error}"))?;
    if feed.items.len() > NEWS_COUNT {
        return Err("Too many news articles.".into());
    }
    for item in &feed.items {
        if item.locale != expected || !valid_slug(&item.slug) {
            return Err("Invalid news language or article slug.".into());
        }
        validate_text(&item.title, 512)?;
        validate_text(&item.summary, 4096)?;
        if !item.published_at.get(..10).is_some_and(valid_date)
            || item.published_at.as_bytes().get(10) != Some(&b'T')
        {
            return Err("Invalid news publication date.".into());
        }
    }
    Ok(feed)
}

fn validate_text(value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > max || value.chars().any(char::is_control)
    {
        return Err("Invalid news text.".into());
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
        && value.len() <= 256
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

    fn article(locale: &str) -> serde_json::Value {
        serde_json::json!({
            "slug": "le-seuil-est-ouvert", "locale": locale,
            "title": "The Threshold is open", "summary": "Join the playtest.",
            "coverImageUrl": "/art/map/playtest.webp", "authorName": "Rivals Beyond",
            "publishedAt": "2026-09-12T07:00:00+00:00", "updatedAt": "2026-09-12T08:00:00+00:00"
        })
    }

    #[test]
    fn articles_request_the_selected_language_from_the_website_backend() {
        for (locale, expected) in [("frFR", "fr"), ("enUS", "en")] {
            let url = news_url(locale).unwrap();
            assert_eq!(
                url.origin().ascii_serialization(),
                "https://api.rivalsbeyond.com"
            );
            assert_eq!(url.path(), "/api/v1/news");
            assert_eq!(
                url.query().unwrap(),
                format!("locale={expected}&page=1&pageSize=3")
            );
        }
        assert!(news_url("deDE").is_err());
    }

    #[test]
    fn backend_articles_preserve_localized_text_and_allow_empty_results() {
        let page = serde_json::json!({"items": [article("en")], "totalCount": 27, "page": 1, "pageSize": 3});
        let feed = load_news(&serde_json::to_vec(&page).unwrap(), "enUS").unwrap();
        assert_eq!(feed.items[0].title, "The Threshold is open");
        assert_eq!(feed.items[0].published_at, "2026-09-12T07:00:00+00:00");
        assert!(load_news(br#"{"items":[]}"#, "frFR")
            .unwrap()
            .items
            .is_empty());
        assert!(load_news(&serde_json::to_vec(&page).unwrap(), "frFR").is_err());
    }

    #[test]
    fn invalid_or_oversized_articles_never_reach_the_interface() {
        for (field, value) in [
            ("slug", "../register"),
            ("title", ""),
            ("publishedAt", "yesterday"),
            ("summary", "bad\ntext"),
        ] {
            let mut item = article("en");
            item[field] = value.into();
            let page = serde_json::to_vec(&serde_json::json!({"items":[item]})).unwrap();
            assert!(load_news(&page, "enUS").is_err(), "{field}");
        }
        assert!(load_news(&vec![b' '; MAX_NEWS_SIZE as usize + 1], "enUS").is_err());
        let page =
            serde_json::to_vec(&serde_json::json!({"items":vec![article("en"); 4]})).unwrap();
        assert!(load_news(&page, "enUS").is_err());
    }
}
