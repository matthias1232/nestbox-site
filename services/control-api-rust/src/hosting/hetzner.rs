//! Hetzner Cloud. The join command rides in `user_data`.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{HostDriver, HostError, HostRequest, HostState, HostType};

pub struct Hetzner {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Hetzner {
    pub fn new(base: &str, token: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().unwrap_or_default(),
        }
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, HostError> {
        let mut request = self.http.request(method, format!("{}{}", self.base, path)).bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|e| HostError::Unavailable(e.to_string()))?;
        let status = response.status();
        let text = response.text().await.map_err(|e| HostError::Unavailable(e.to_string()))?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status.as_u16() == 404 {
            return Err(HostError::Invalid("not found".into()));
        }
        if !status.is_success() {
            let message = value["error"]["message"].as_str().unwrap_or("the request failed");
            return Err(HostError::Rejected(format!("{status}: {message}")));
        }
        Ok(value)
    }
}

/// The request body for a new server. Pure, so the shape is tested without a network.
pub fn server_body(request: &HostRequest) -> Value {
    let mut body = json!({
        "name": request.name,
        "server_type": request.host_type,
        "image": "ubuntu-24.04",
        "user_data": request.user_data,
        "start_after_create": true,
        "labels": { "managed-by": "nestbox" },
    });
    if let Some(location) = &request.location {
        body["location"] = json!(location);
    }
    body
}

/// Server types out of the API answer, with the hourly price in thousandths.
pub fn parse_types(value: &Value) -> Vec<HostType> {
    value["server_types"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|t| t["deprecated"].as_bool() != Some(true))
                .filter_map(|t| {
                    let prices = t["prices"].as_array()?;
                    let hourly = |price: &Value| price["price_hourly"]["net"].as_str().and_then(|p| p.parse::<f64>().ok());
                    let cheapest = prices.iter().filter_map(hourly).fold(None, |best: Option<f64>, p| Some(best.map_or(p, |b| b.min(p))));
                    Some(HostType {
                        name: t["name"].as_str()?.to_string(),
                        cores: t["cores"].as_u64()? as u32,
                        memory_mb: (t["memory"].as_f64()? * 1024.0) as u64,
                        disk_gb: t["disk"].as_u64()?,
                        hourly_milli: cheapest.map(|p| (p * 1000.0).round() as u64),
                        locations: prices.iter().filter_map(|p| p["location"].as_str().map(str::to_string)).collect(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn state_of(server: &Value) -> HostState {
    HostState {
        external_id: server["id"].as_u64().map(|i| i.to_string()).unwrap_or_default(),
        state: match server["status"].as_str() {
            Some("running") => "running",
            Some("deleting") | Some("off") => "gone",
            _ => "creating",
        }
        .to_string(),
        address: server["public_net"]["ipv4"]["ip"].as_str().map(str::to_string),
    }
}

#[async_trait]
impl HostDriver for Hetzner {
    fn kind(&self) -> &'static str {
        "hetzner"
    }

    async fn types(&self) -> Result<Vec<HostType>, HostError> {
        let value = self.call(reqwest::Method::GET, "/server_types?per_page=50", None).await?;
        Ok(parse_types(&value))
    }

    async fn create(&self, request: &HostRequest) -> Result<HostState, HostError> {
        let value = self.call(reqwest::Method::POST, "/servers", Some(server_body(request))).await?;
        Ok(state_of(&value["server"]))
    }

    async fn status(&self, external_id: &str) -> Result<HostState, HostError> {
        match self.call(reqwest::Method::GET, &format!("/servers/{external_id}"), None).await {
            Ok(value) => Ok(state_of(&value["server"])),
            Err(HostError::Invalid(_)) => Ok(HostState { external_id: external_id.into(), state: "gone".into(), address: None }),
            Err(other) => Err(other),
        }
    }

    async fn destroy(&self, external_id: &str) -> Result<(), HostError> {
        match self.call(reqwest::Method::DELETE, &format!("/servers/{external_id}"), None).await {
            Ok(_) | Err(HostError::Invalid(_)) => Ok(()),
            Err(other) => Err(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_body_carries_the_join_command_as_user_data() {
        let body = server_body(&HostRequest {
            name: "gpu-1".into(),
            host_type: "cx22".into(),
            location: Some("fsn1".into()),
            user_data: "#cloud-config\nruncmd: []\n".into(),
            join_command: String::new(),
        });
        assert_eq!(body["server_type"], "cx22");
        assert_eq!(body["location"], "fsn1");
        assert!(body["user_data"].as_str().unwrap_or_default().starts_with("#cloud-config"));
        assert_eq!(body["start_after_create"], true);
    }

    #[test]
    fn types_are_priced_by_the_hour_in_thousandths_and_deprecated_ones_are_dropped() {
        let answer = json!({"server_types": [
            {"name": "cx22", "cores": 2, "memory": 4.0, "disk": 40, "deprecated": false,
             "prices": [{"location": "fsn1", "price_hourly": {"net": "0.0060"}}, {"location": "nbg1", "price_hourly": {"net": "0.0072"}}]},
            {"name": "old", "cores": 1, "memory": 1.0, "disk": 10, "deprecated": true, "prices": []}
        ]});
        let types = parse_types(&answer);
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].hourly_milli, Some(6));
        assert_eq!(types[0].memory_mb, 4096);
        assert_eq!(types[0].locations, vec!["fsn1", "nbg1"]);
    }

    #[test]
    fn a_server_answer_maps_to_a_state() {
        let running = state_of(&json!({"id": 42, "status": "running", "public_net": {"ipv4": {"ip": "203.0.113.9"}}}));
        assert_eq!((running.external_id.as_str(), running.state.as_str(), running.address.as_deref()), ("42", "running", Some("203.0.113.9")));
        assert_eq!(state_of(&json!({"id": 1, "status": "initializing"})).state, "creating");
    }
}
