//! Vultr. The join command rides in the instance's base64 `user_data`.
//!
//! Shapes follow Vultr's published v2 API and are unit tested; this driver has not been run against
//! a live account. The operating system id defaults to Ubuntu 24.04 x64 (2284) and can be set in the
//! account's settings (`os_id`) if Vultr renumbers it.

use async_trait::async_trait;
use base64::Engine;
use serde_json::{Value, json};

use super::{HostDriver, HostError, HostRequest, HostState, HostType};

pub struct Vultr {
    base: String,
    token: String,
    os_id: u64,
    http: reqwest::Client,
}

impl Vultr {
    pub fn new(base: &str, token: &str, os_id: u64) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            os_id,
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
            return Err(HostError::Rejected(format!("{status}: {}", value["error"].as_str().unwrap_or("the request failed"))));
        }
        Ok(value)
    }
}

pub fn instance_body(request: &HostRequest, os_id: u64) -> Value {
    json!({
        "region": request.location.clone().unwrap_or_else(|| "fra".into()),
        "plan": request.host_type,
        "os_id": os_id,
        "label": request.name,
        "hostname": request.name,
        "user_data": base64::engine::general_purpose::STANDARD.encode(request.user_data.as_bytes()),
        "tags": ["nestbox"],
    })
}

pub fn parse_plans(value: &Value) -> Vec<HostType> {
    value["plans"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|p| {
                    // Vultr publishes a monthly price; the hourly one is that over 730 hours when
                    // no hourly figure is given.
                    let hourly = p["hourly_cost"].as_f64().or_else(|| p["monthly_cost"].as_f64().map(|m| m / 730.0));
                    Some(HostType {
                        name: p["id"].as_str()?.to_string(),
                        cores: p["vcpu_count"].as_u64()? as u32,
                        memory_mb: p["ram"].as_u64()?,
                        disk_gb: p["disk"].as_u64().unwrap_or(0),
                        hourly_milli: hourly.map(|h| (h * 1000.0).round() as u64),
                        locations: p["locations"].as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn state_of(instance: &Value) -> HostState {
    let address = instance["main_ip"].as_str().filter(|ip| *ip != "0.0.0.0").map(str::to_string);
    HostState {
        external_id: instance["id"].as_str().unwrap_or_default().to_string(),
        state: if instance["status"].as_str() == Some("active") && instance["power_status"].as_str() == Some("running") {
            "running"
        } else {
            "creating"
        }
        .to_string(),
        address,
    }
}

#[async_trait]
impl HostDriver for Vultr {
    fn kind(&self) -> &'static str {
        "vultr"
    }
    async fn types(&self) -> Result<Vec<HostType>, HostError> {
        Ok(parse_plans(&self.call(reqwest::Method::GET, "/plans?type=vc2,vhf,vdc&per_page=200", None).await?))
    }
    async fn create(&self, request: &HostRequest) -> Result<HostState, HostError> {
        let value = self.call(reqwest::Method::POST, "/instances", Some(instance_body(request, self.os_id))).await?;
        Ok(state_of(&value["instance"]))
    }
    async fn status(&self, external_id: &str) -> Result<HostState, HostError> {
        match self.call(reqwest::Method::GET, &format!("/instances/{external_id}"), None).await {
            Ok(value) => Ok(state_of(&value["instance"])),
            Err(HostError::Invalid(_)) => Ok(HostState { external_id: external_id.into(), state: "gone".into(), address: None }),
            Err(other) => Err(other),
        }
    }
    async fn destroy(&self, external_id: &str) -> Result<(), HostError> {
        match self.call(reqwest::Method::DELETE, &format!("/instances/{external_id}"), None).await {
            Ok(_) | Err(HostError::Invalid(_)) => Ok(()),
            Err(other) => Err(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_data_is_base64_and_the_os_comes_from_the_account() {
        let body = instance_body(&HostRequest { name: "n1".into(), host_type: "vc2-2c-4gb".into(), location: Some("ams".into()), user_data: "#cloud-config\n".into(), join_command: String::new() }, 2284);
        assert_eq!(body["os_id"], 2284);
        assert_eq!(body["region"], "ams");
        let decoded = base64::engine::general_purpose::STANDARD.decode(body["user_data"].as_str().unwrap_or_default()).expect("base64");
        assert_eq!(decoded, b"#cloud-config\n");
    }

    #[test]
    fn plans_are_priced_hourly_or_from_the_monthly_figure() {
        let answer = json!({"plans": [
            {"id": "vc2-2c-4gb", "vcpu_count": 2, "ram": 4096, "disk": 80, "monthly_cost": 24.0, "locations": ["ams", "fra"]},
            {"id": "vhf-1c-1gb", "vcpu_count": 1, "ram": 1024, "disk": 32, "hourly_cost": 0.009, "monthly_cost": 6.0}
        ]});
        let plans = parse_plans(&answer);
        assert_eq!(plans[0].hourly_milli, Some(33));
        assert_eq!(plans[1].hourly_milli, Some(9));
    }

    #[test]
    fn an_instance_is_running_only_when_active_and_powered_on() {
        assert_eq!(state_of(&json!({"id": "a", "status": "active", "power_status": "running", "main_ip": "203.0.113.5"})).state, "running");
        let pending = state_of(&json!({"id": "a", "status": "pending", "power_status": "stopped", "main_ip": "0.0.0.0"}));
        assert_eq!((pending.state.as_str(), pending.address), ("creating", None));
    }
}
