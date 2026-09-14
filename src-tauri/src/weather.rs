//! QWeather (和风天气) client.
//!
//! Requests are issued from the Rust side so the webview never has to deal
//! with CORS and the API key is not exposed in the front-end bundle.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

const GEO_HOST: &str = "https://geoapi.qweather.com";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WeatherNow {
    pub obs_time: String,
    pub temp: String,
    pub feels_like: String,
    pub icon: String,
    pub text: String,
    pub wind_dir: String,
    pub wind_scale: String,
    pub humidity: String,
    pub precip: String,
    pub pressure: String,
    pub vis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WeatherDaily {
    pub fx_date: String,
    pub temp_max: String,
    pub temp_min: String,
    pub icon_day: String,
    pub text_day: String,
    pub icon_night: String,
    pub text_night: String,
    pub sunrise: String,
    pub sunset: String,
    pub precip: String,
    pub uv_index: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WeatherAdvice {
    /// e.g. 交通指数
    pub name: String,
    /// e.g. 良好
    pub category: String,
    /// the full sentence rendered under the current conditions
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WeatherPayload {
    pub location_name: String,
    pub now: WeatherNow,
    pub daily: Vec<WeatherDaily>,
    pub update_time: String,
    /// Optional lifestyle line. Missing whenever the plan cannot query the
    /// indices endpoint - the panel simply renders without it.
    pub advice: Option<WeatherAdvice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GeoCity {
    pub id: String,
    pub name: String,
    pub adm1: String,
    pub adm2: String,
    pub country: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NowResponse {
    code: String,
    #[serde(default)]
    update_time: Option<String>,
    #[serde(default)]
    now: Option<WeatherNow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyResponse {
    code: String,
    #[serde(default)]
    daily: Option<Vec<WeatherDaily>>,
}

#[derive(Deserialize)]
struct LookupResponse {
    code: String,
    #[serde(default)]
    location: Option<Vec<GeoCity>>,
}

/// `/v7/indices/1d` - we only keep `daily[0]`, which is what the widget shows.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndicesResponse {
    code: String,
    #[serde(default)]
    daily: Option<Vec<WeatherAdvice>>,
}

/// 交通指数 (QWeather index type 15). Its `text` is a full sentence such as
/// "天气较好，路面干燥，交通气象条件良好，车辆可以正常行驶。" - the one line
/// of prose the weather card shows under the temperature.
const TRAFFIC_INDEX: &str = "15";

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent("WSight/0.1")
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败: {e}"))
}

fn qweather_error(code: &str) -> String {
    match code {
        "204" => "该地区暂无数据".to_string(),
        "400" => "请求参数错误".to_string(),
        "401" => "API Key 无效或未通过认证".to_string(),
        "402" => "超过访问次数或余额不足".to_string(),
        "403" => "无权限访问该数据".to_string(),
        "404" => "查询的城市不存在".to_string(),
        "429" => "请求过于频繁，已被限流".to_string(),
        _ => format!("和风天气返回错误码 {code}"),
    }
}

/// Fetch weather using the persisted configuration.
pub async fn fetch(cfg: &AppConfig) -> Result<WeatherPayload, String> {
    fetch_with(
        &cfg.qweather_host,
        &cfg.qweather_key,
        &cfg.location_id,
        &cfg.location_name,
    )
    .await
}

/// Fetch weather from ad-hoc values that have not been saved yet - used by the
/// "测试连接" button in the settings window.
pub async fn probe(host: &str, key: &str, location_id: &str) -> Result<WeatherPayload, String> {
    fetch_with(host, key, location_id, "").await
}

pub async fn fetch_with(
    host: &str,
    key: &str,
    location_id: &str,
    name_hint: &str,
) -> Result<WeatherPayload, String> {
    if key.trim().is_empty() {
        return Err("未配置和风天气 API Key".to_string());
    }
    if location_id.trim().is_empty() {
        return Err("未配置城市（LocationID）".to_string());
    }

    let host = crate::config::normalize_host(host);
    let host = host.as_str();
    let key = key.trim();
    let loc = location_id.trim();
    let http = client()?;

    let now_url = format!("{host}/v7/weather/now?location={loc}&key={key}");
    let daily_url = format!("{host}/v7/weather/7d?location={loc}&key={key}");
    let advice_url =
        format!("{host}/v7/indices/1d?type={TRAFFIC_INDEX}&location={loc}&key={key}");

    // Three independent GETs - fire them together so the panel waits for one
    // round trip instead of three.
    let (now_res, daily_res, advice_res) = tokio::join!(
        fetch_json::<NowResponse>(&http, &now_url, "实时天气"),
        fetch_json::<DailyResponse>(&http, &daily_url, "预报"),
        fetch_json::<IndicesResponse>(&http, &advice_url, "生活指数"),
    );

    let now_resp = now_res?;
    if now_resp.code != "200" {
        return Err(qweather_error(&now_resp.code));
    }

    // Forecast and indices are best-effort. The indices endpoint is not part
    // of every subscription, and a forecast hiccup must not blank the card.
    let daily = match daily_res {
        Ok(resp) if resp.code == "200" => resp.daily.unwrap_or_default(),
        _ => Vec::new(),
    };

    let advice = match advice_res {
        Ok(resp) if resp.code == "200" => resp
            .daily
            .unwrap_or_default()
            .into_iter()
            .find(|a| !a.text.trim().is_empty()),
        _ => None,
    };

    let name = if name_hint.trim().is_empty() {
        loc.to_string()
    } else {
        name_hint.trim().to_string()
    };

    Ok(WeatherPayload {
        location_name: name,
        now: now_resp.now.unwrap_or_default(),
        daily,
        update_time: now_resp.update_time.unwrap_or_default(),
        advice,
    })
}

async fn fetch_json<T: for<'de> Deserialize<'de>>(
    http: &reqwest::Client,
    url: &str,
    what: &str,
) -> Result<T, String> {
    http.get(url)
        .send()
        .await
        .map_err(|e| format!("{what}请求失败: {e}"))?
        .json::<T>()
        .await
        .map_err(|e| format!("{what}解析失败: {e}"))
}

pub async fn lookup_city(key: &str, keyword: &str) -> Result<Vec<GeoCity>, String> {
    if key.trim().is_empty() {
        return Err("未配置和风天气 API Key".to_string());
    }
    if keyword.trim().is_empty() {
        return Err("请输入城市名".to_string());
    }

    let url = format!(
        "{GEO_HOST}/v2/city/lookup?location={}&key={}&number=10",
        urlencoding(keyword.trim()),
        key.trim()
    );

    let resp = client()?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("城市查询失败: {e}"))?
        .json::<LookupResponse>()
        .await
        .map_err(|e| format!("城市查询解析失败: {e}"))?;

    if resp.code != "200" {
        return Err(qweather_error(&resp.code));
    }
    Ok(resp.location.unwrap_or_default())
}

/// Minimal percent-encoding for the query value. Avoids pulling in a crate
/// just for Chinese city names.
fn urlencoding(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}
