//! DigitalOcean. The join command rides in the droplet's `user_data`.
//!
//! Request and answer shapes follow DigitalOcean's published v2 API and are unit tested; this driver
//! has not been run against a live account.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{HostDriver, HostError, HostRequest, HostState, HostType};

pub struct DigitalOcean {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl DigitalOcean {
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
            return Err(HostError::Rejected(format!("{status}: {}", value["message"].as_str().unwrap_or("the request failed"))));
        }
        Ok(value)
    }
}

pub fn droplet_body(request: &HostRequest) -> Value {
    json!({
        "name": request.name,
        "region": request.location.clone().unwrap_or_else(|| "fra1".into()),
        "size": request.host_type,
        "image": "ubuntu-24-04-x64",
        "user_data": request.user_data,
        "tags": ["nestbox"],
    })
}

pub fn parse_sizes(value: &Value) -> Vec<HostType> {
    value["sizes"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|s| s["available"].as_bool() != Some(false))
                .filter_map(|s| {
                    Some(HostType {
                        name: s["slug"].as_str()?.to_string(),
                        cores: s["vcpus"].as_u64()? as u32,
                        memory_mb: s["memory"].as_u64()?,
                        disk_gb: s["disk"].as_u64().unwrap_or(0),
                        hourly_milli: s["price_hourly"].as_f64().map(|p| (p * 1000.0).round() as u64),
                        locations: s["regions"].as_array().map(|r| r.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn state_of(droplet: &Value) -> HostState {
    let address = droplet["networks"]["v4"]
        .as_array()
        .and_then(|list| list.iter().find(|n| n["type"].as_str() == Some("public")))
        .and_then(|n| n["ip_address"].as_str())
        .map(str::to_string);
    HostState {
        external_id: droplet["id"].as_u64().map(|i| i.to_string()).unwrap_or_default(),
        state: match droplet["status"].as_str() {
            Some("active") => "running",
            Some("off") | Some("archive") => "gone",
            _ => "creating",
        }
        .to_string(),
        address,
    }
}

#[async_trait]
impl HostDriver for DigitalOcean {
    fn kind(&self) -> &'static str {
        "digitalocean"
    }
    async fn types(&self) -> Result<Vec<HostType>, HostError> {
        Ok(parse_sizes(&self.call(reqwest::Method::GET, "/sizes?per_page=200", None).await?))
    }
    async fn create(&self, request: &HostRequest) -> Result<HostState, HostError> {
        let value = self.call(reqwest::Method::POST, "/droplets", Some(droplet_body(request))).await?;
        Ok(state_of(&value["droplet"]))
    }
    async fn status(&self, external_id: &str) -> Result<HostState, HostError> {
        match self.call(reqwest::Method::GET, &format!("/droplets/{external_id}"), None).await {
            Ok(value) => Ok(state_of(&value["droplet"])),
            Err(HostError::Invalid(_)) => Ok(HostState { external_id: external_id.into(), state: "gone".into(), address: None }),
            Err(other) => Err(other),
        }
    }
    async fn destroy(&self, external_id: &str) -> Result<(), HostError> {
        match self.call(reqwest::Method::DELETE, &format!("/droplets/{external_id}"), None).await {
            Ok(_) | Err(HostError::Invalid(_)) => Ok(()),
            Err(other) => Err(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_droplet_body_carries_the_join_command_and_a_default_region() {
        let body = droplet_body(&HostRequest { name: "n1".into(), host_type: "s-2vcpu-4gb".into(), location: None, user_data: "#cloud-config\n".into(), join_command: String::new() });
        assert_eq!(body["size"], "s-2vcpu-4gb");
        assert_eq!(body["region"], "fra1");
        assert_eq!(body["image"], "ubuntu-24-04-x64");
        assert!(body["user_data"].as_str().unwrap_or_default().starts_with("#cloud-config"));
    }

    #[test]
    fn sizes_are_priced_in_thousandths_and_unavailable_ones_dropped() {
        let answer = json!({"sizes": [
            {"slug": "s-2vcpu-4gb", "vcpus": 2, "memory": 4096, "disk": 80, "price_hourly": 0.02679, "available": true, "regions": ["fra1", "ams3"]},
            {"slug": "gone", "vcpus": 1, "memory": 1024, "disk": 10, "price_hourly": 0.01, "available": false, "regions": []}
        ]});
        let sizes = parse_sizes(&answer);
        assert_eq!(sizes.len(), 1);
        assert_eq!(sizes[0].hourly_milli, Some(27));
        assert_eq!(sizes[0].locations, vec!["fra1", "ams3"]);
    }

    #[test]
    fn a_droplet_answer_maps_to_a_state_with_its_public_address() {
        let d = json!({"id": 7, "status": "active", "networks": {"v4": [{"type": "private", "ip_address": "10.0.0.2"}, {"type": "public", "ip_address": "203.0.113.4"}]}});
        let state = state_of(&d);
        assert_eq!((state.state.as_str(), state.address.as_deref()), ("running", Some("203.0.113.4")));
        assert_eq!(state_of(&json!({"id": 1, "status": "new"})).state, "creating");
    }
}
