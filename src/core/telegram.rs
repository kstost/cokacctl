use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::core::config::TokenBotInfo;

const TELEGRAM_API_BASE: &str = "https://api.telegram.org";

#[derive(Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
    error_code: Option<i64>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct TelegramUser {
    id: i64,
    #[serde(default)]
    is_bot: bool,
    first_name: String,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Deserialize)]
struct TelegramShortDescription {
    #[serde(default)]
    short_description: String,
}

#[derive(Deserialize)]
struct TelegramDescription {
    #[serde(default)]
    description: String,
}

pub async fn fetch_bot_info(token: &str) -> Result<TokenBotInfo, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .user_agent(format!("cokacctl/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "Cannot initialize Telegram API client".to_string())?;

    let user: TelegramUser = telegram_api(&client, token, "getMe").await?;
    if !user.is_bot {
        return Err("Telegram token is valid, but it does not belong to a bot".into());
    }

    let short_description =
        match telegram_api::<TelegramShortDescription>(&client, token, "getMyShortDescription")
            .await
        {
            Ok(v) => non_empty(v.short_description),
            Err(e) => {
                dlog!("telegram", "getMyShortDescription skipped: {}", e);
                None
            }
        };
    let description =
        match telegram_api::<TelegramDescription>(&client, token, "getMyDescription").await {
            Ok(v) => non_empty(v.description),
            Err(e) => {
                dlog!("telegram", "getMyDescription skipped: {}", e);
                None
            }
        };

    Ok(TokenBotInfo {
        id: Some(user.id),
        first_name: non_empty(user.first_name),
        username: user.username.and_then(non_empty),
        short_description,
        description,
    })
}

async fn telegram_api<T: DeserializeOwned>(
    client: &reqwest::Client,
    token: &str,
    method: &str,
) -> Result<T, String> {
    let url = format!("{}/bot{}/{}", TELEGRAM_API_BASE, token, method);
    let response = client.get(url).send().await.map_err(|e| {
        if e.is_timeout() {
            "Telegram Bot API request timed out".to_string()
        } else if e.is_connect() {
            "Cannot connect to Telegram Bot API".to_string()
        } else {
            "Telegram Bot API request failed".to_string()
        }
    })?;
    let http_status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|_| "Cannot read Telegram Bot API response".to_string())?;
    let parsed: TelegramResponse<T> = serde_json::from_slice(&body).map_err(|_| {
        if http_status.is_success() {
            "Unexpected Telegram Bot API response".to_string()
        } else {
            format!("Telegram Bot API returned HTTP {}", http_status.as_u16())
        }
    })?;

    if parsed.ok {
        return parsed
            .result
            .ok_or_else(|| "Telegram Bot API response did not include a result".to_string());
    }

    Err(telegram_api_error(method, parsed.error_code, parsed.description))
}

fn telegram_api_error(method: &str, code: Option<i64>, description: Option<String>) -> String {
    let desc = description
        .map(|d| sanitize_control_chars(&d))
        .filter(|d| !d.trim().is_empty());
    match (method, code, desc) {
        ("getMe", Some(401), _) => "Telegram rejected this bot token".into(),
        (_, Some(code), Some(desc)) => format!("Telegram Bot API error {}: {}", code, desc),
        (_, Some(code), None) => format!("Telegram Bot API error {}", code),
        (_, None, Some(desc)) => format!("Telegram Bot API error: {}", desc),
        _ => "Telegram Bot API request was rejected".into(),
    }
}

fn sanitize_control_chars(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

fn non_empty(s: String) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
