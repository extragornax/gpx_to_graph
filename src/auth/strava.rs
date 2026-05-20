use serde::Deserialize;

pub struct StravaConfig {
    pub client_id: String,
    pub client_secret: String,
    pub base_url: String,
    pub webhook_verify_token: String,
}

impl StravaConfig {
    pub fn from_env() -> Option<Self> {
        let client_id = std::env::var("STRAVA_CLIENT_ID").ok()?;
        let client_secret = std::env::var("STRAVA_CLIENT_SECRET").ok()?;
        let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:3000".into());
        let webhook_verify_token =
            std::env::var("STRAVA_WEBHOOK_VERIFY_TOKEN").unwrap_or_else(|_| {
                format!(
                    "col-verify-{}",
                    &client_secret[..8.min(client_secret.len())]
                )
            });
        Some(Self {
            client_id,
            client_secret,
            base_url,
            webhook_verify_token,
        })
    }

    pub fn authorize_url(&self, redirect_after: Option<&str>) -> String {
        let state = redirect_after.unwrap_or("/");
        let encoded: String = state
            .bytes()
            .flat_map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                    vec![b as char]
                }
                _ => format!("%{b:02X}").chars().collect(),
            })
            .collect();
        format!(
            "https://www.strava.com/oauth/authorize?client_id={}&redirect_uri={}/auth/strava/callback&response_type=code&scope=activity:read_all&approval_prompt=auto&state={}",
            self.client_id, self.base_url, encoded,
        )
    }
}

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub athlete: Athlete,
}

#[derive(Deserialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[derive(Deserialize)]
pub struct Athlete {
    pub id: i64,
    pub firstname: Option<String>,
    pub lastname: Option<String>,
}

pub async fn exchange_code(config: &StravaConfig, code: &str) -> anyhow::Result<TokenResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://www.strava.com/oauth/token")
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("code", code),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<TokenResponse>()
        .await?;
    Ok(resp)
}

pub async fn refresh_token(
    config: &StravaConfig,
    refresh: &str,
) -> anyhow::Result<RefreshResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://www.strava.com/oauth/token")
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<RefreshResponse>()
        .await?;
    Ok(resp)
}

pub struct StreamPoint {
    pub distance_km: f64,
    pub elevation: f64,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct StravaActivity {
    pub id: i64,
    pub name: String,
    pub start_date_local: String,
    #[serde(rename = "type")]
    pub activity_type: String,
    pub total_elevation_gain: Option<f64>,
}

pub async fn fetch_activities(
    access_token: &str,
    page: u32,
) -> anyhow::Result<Vec<StravaActivity>> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://www.strava.com/api/v3/athlete/activities")
        .bearer_auth(access_token)
        .query(&[("per_page", "200"), ("page", &page.to_string())])
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<StravaActivity>>()
        .await?;
    Ok(resp)
}

#[derive(Deserialize)]
struct StreamEntry {
    #[serde(rename = "type")]
    stream_type: String,
    data: serde_json::Value,
}

pub async fn fetch_streams(
    access_token: &str,
    activity_id: i64,
) -> anyhow::Result<Option<Vec<StreamPoint>>> {
    let client = reqwest::Client::new();
    let url = format!("https://www.strava.com/api/v3/activities/{activity_id}/streams");
    let resp = client
        .get(&url)
        .bearer_auth(access_token)
        .query(&[
            ("keys", "latlng,altitude,distance"),
            ("key_type", "distance"),
        ])
        .send()
        .await?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let resp = resp.error_for_status()?;
    let streams: Vec<StreamEntry> = resp.json().await?;

    let mut latlng: Option<Vec<[f64; 2]>> = None;
    let mut altitude: Option<Vec<f64>> = None;
    let mut distance: Option<Vec<f64>> = None;

    for s in streams {
        match s.stream_type.as_str() {
            "latlng" => latlng = serde_json::from_value(s.data).ok(),
            "altitude" => altitude = serde_json::from_value(s.data).ok(),
            "distance" => distance = serde_json::from_value(s.data).ok(),
            _ => {}
        }
    }

    let (Some(ll), Some(alt), Some(dist)) = (latlng, altitude, distance) else {
        return Ok(None);
    };

    if ll.len() != alt.len() || ll.len() != dist.len() {
        return Ok(None);
    }

    let points: Vec<StreamPoint> = ll
        .iter()
        .zip(alt.iter())
        .zip(dist.iter())
        .map(|((coord, &ele), &d)| StreamPoint {
            distance_km: d / 1000.0,
            elevation: ele,
            lat: coord[0],
            lon: coord[1],
        })
        .collect();

    Ok(Some(points))
}

pub async fn fetch_activity(
    access_token: &str,
    activity_id: i64,
) -> anyhow::Result<Option<StravaActivity>> {
    let client = reqwest::Client::new();
    let url = format!("https://www.strava.com/api/v3/activities/{activity_id}");
    let resp = client.get(&url).bearer_auth(access_token).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(resp.error_for_status()?.json().await?))
}

#[derive(Deserialize, Debug)]
pub struct WebhookSubscription {
    pub id: i64,
    pub callback_url: String,
}

pub fn webhook_callback_url(config: &StravaConfig) -> String {
    format!(
        "{}/col/webhook/strava",
        config.base_url.trim_end_matches('/')
    )
}

pub async fn list_subscriptions(
    config: &StravaConfig,
) -> anyhow::Result<Vec<WebhookSubscription>> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://www.strava.com/api/v3/push_subscriptions")
        .query(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
        ])
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "list push_subscriptions failed [{status}]: {body}"
        ));
    }
    Ok(serde_json::from_str(&body)?)
}

pub async fn delete_subscription(config: &StravaConfig, id: i64) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = format!("https://www.strava.com/api/v3/push_subscriptions/{id}");
    let resp = client
        .delete(&url)
        .query(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
        ])
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "delete push_subscription {id} failed [{status}]: {body}"
        ));
    }
    Ok(())
}

pub async fn create_subscription(
    config: &StravaConfig,
    callback_url: &str,
) -> anyhow::Result<i64> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://www.strava.com/api/v3/push_subscriptions")
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("callback_url", callback_url),
            ("verify_token", config.webhook_verify_token.as_str()),
        ])
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "create push_subscription failed [{status}]: {body}"
        ));
    }
    let v: serde_json::Value = serde_json::from_str(&body)?;
    v.get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("missing id in response: {body}"))
}

/// Ensure exactly one Strava webhook subscription points at our callback.
/// Strava allows only one subscription per app, so any stale subscription
/// with a different callback_url is deleted before creating a fresh one.
pub async fn ensure_subscription(config: &StravaConfig) -> anyhow::Result<i64> {
    let callback_url = webhook_callback_url(config);
    let existing = list_subscriptions(config).await?;
    if let Some(sub) = existing.iter().find(|s| s.callback_url == callback_url) {
        return Ok(sub.id);
    }
    for sub in existing {
        delete_subscription(config, sub.id).await?;
    }
    create_subscription(config, &callback_url).await
}
