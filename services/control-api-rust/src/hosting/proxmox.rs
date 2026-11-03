//! Proxmox VE: clone a prepared template, give the clone an address, start it, and run the join
//! command inside it through the QEMU guest agent.
//!
//! The template is the operator's one-time preparation (`deploy/scripts/make-proxmox-template.sh`):
//! an Ubuntu cloud image with `qemu-guest-agent` installed and enabled. User data cannot be handed
//! to a clone through the API (a `cicustom` snippet is a file on the host), which is why the guest
//! agent runs the command instead.

use async_trait::async_trait;
use serde_json::Value;

use super::{HostDriver, HostError, HostRequest, HostState, HostType};

pub struct Proxmox {
    base: String,
    token: String,
    node: String,
    template: u32,
    pool: Vec<String>,
    gateway: String,
    prefix: u8,
    nameserver: String,
    types: Vec<HostType>,
    http: reqwest::Client,
}

/// Percent-encode a form value.
pub fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A form body from key/value pairs; a key may repeat (the agent's `command` list does).
pub fn form(pairs: &[(&str, String)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect::<Vec<_>>().join("&")
}

/// `192.168.55.60-192.168.55.69` as the list of addresses it names.
pub fn expand_pool(range: &str) -> Result<Vec<String>, HostError> {
    let bad = || HostError::Invalid("The address pool is two IPv4 addresses with a dash, for example 192.168.55.60-192.168.55.69.".into());
    let (from, to) = range.split_once('-').ok_or_else(bad)?;
    let parse = |text: &str| -> Result<[u8; 4], HostError> {
        let parts: Vec<u8> = text.trim().split('.').map(|p| p.parse::<u8>()).collect::<Result<_, _>>().map_err(|_| bad())?;
        <[u8; 4]>::try_from(parts).map_err(|_| bad())
    };
    let (a, b) = (parse(from)?, parse(to)?);
    if a[..3] != b[..3] || a[3] > b[3] {
        return Err(bad());
    }
    Ok((a[3]..=b[3]).map(|last| format!("{}.{}.{}.{last}", a[0], a[1], a[2])).collect())
}

/// The first pool address no existing machine uses.
pub fn free_address(pool: &[String], used: &[String]) -> Option<String> {
    pool.iter().find(|candidate| !used.iter().any(|u| u == *candidate)).cloned()
}

/// The address out of a config line such as `ip=192.168.55.61/24,gw=192.168.55.1`.
pub fn address_of_ipconfig(line: &str) -> Option<String> {
    line.split(',').find_map(|part| part.strip_prefix("ip=")).and_then(|ip| ip.split('/').next()).map(str::to_string)
}

impl Proxmox {
    pub fn from_config(config: &Value, token: &str) -> Result<Self, HostError> {
        let text = |key: &str| config.get(key).and_then(|v| v.as_str()).map(str::to_string);
        let need = |key: &str| text(key).ok_or_else(|| HostError::Invalid(format!("The Proxmox account needs '{key}'.")));
        let verify = config.get("verify_tls").and_then(|v| v.as_bool()).unwrap_or(false);
        let types = config
            .get("types")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|t| {
                        Some(HostType {
                            name: t["name"].as_str()?.to_string(),
                            cores: t["cores"].as_u64()? as u32,
                            memory_mb: t["memory_mb"].as_u64()?,
                            disk_gb: t["disk_gb"].as_u64().unwrap_or(0),
                            hourly_milli: Some(0),
                            locations: Vec::new(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                vec![HostType { name: "small".into(), cores: 2, memory_mb: 3072, disk_gb: 0, hourly_milli: Some(0), locations: Vec::new() }]
            });
        Ok(Self {
            base: need("api_url")?.trim_end_matches('/').to_string(),
            token: token.to_string(),
            node: need("node")?,
            template: config.get("template_vmid").and_then(|v| v.as_u64()).ok_or_else(|| HostError::Invalid("The Proxmox account needs 'template_vmid'.".into()))? as u32,
            pool: expand_pool(&need("address_pool")?)?,
            gateway: need("gateway")?,
            prefix: config.get("prefix").and_then(|v| v.as_u64()).unwrap_or(24) as u8,
            nameserver: text("nameserver").unwrap_or_else(|| "1.1.1.1".into()),
            types,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .danger_accept_invalid_certs(!verify)
                .build()
                .map_err(|e| HostError::Invalid(e.to_string()))?,
        })
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<String>) -> Result<Value, HostError> {
        let mut request = self
            .http
            .request(method, format!("{}/api2/json{path}", self.base))
            .header("Authorization", format!("PVEAPIToken={}", self.token));
        if let Some(body) = body {
            request = request.header("content-type", "application/x-www-form-urlencoded").body(body);
        }
        let response = request.send().await.map_err(|e| HostError::Unavailable(e.to_string()))?;
        let status = response.status();
        let text = response.text().await.map_err(|e| HostError::Unavailable(e.to_string()))?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let detail = value["message"].as_str().unwrap_or(status.canonical_reason().unwrap_or("failed"));
            return Err(HostError::Rejected(format!("{status}: {detail}")));
        }
        Ok(value["data"].clone())
    }

    async fn wait_task(&self, upid: &str, seconds: u64) -> Result<(), HostError> {
        let path = format!("/nodes/{}/tasks/{}/status", self.node, encode(upid));
        for _ in 0..seconds / 2 {
            let status = self.call(reqwest::Method::GET, &path, None).await?;
            if status["status"].as_str() == Some("stopped") {
                return if status["exitstatus"].as_str() == Some("OK") {
                    Ok(())
                } else {
                    Err(HostError::Rejected(format!("the task ended with {}", status["exitstatus"])))
                };
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        Err(HostError::Unavailable("the task took too long".into()))
    }

    async fn used_addresses(&self) -> Vec<String> {
        let mut used = Vec::new();
        if let Ok(list) = self.call(reqwest::Method::GET, &format!("/nodes/{}/qemu", self.node), None).await {
            for vm in list.as_array().cloned().unwrap_or_default() {
                let Some(id) = vm["vmid"].as_u64() else { continue };
                if let Ok(config) = self.call(reqwest::Method::GET, &format!("/nodes/{}/qemu/{id}/config", self.node), None).await {
                    for key in ["ipconfig0", "ipconfig1"] {
                        if let Some(address) = config[key].as_str().and_then(address_of_ipconfig) {
                            used.push(address);
                        }
                    }
                }
            }
        }
        used
    }

}

#[async_trait]
impl HostDriver for Proxmox {
    fn kind(&self) -> &'static str {
        "proxmox"
    }

    async fn types(&self) -> Result<Vec<HostType>, HostError> {
        Ok(self.types.clone())
    }

    async fn create(&self, request: &HostRequest) -> Result<HostState, HostError> {
        let shape = self
            .types
            .iter()
            .find(|t| t.name == request.host_type)
            .ok_or_else(|| HostError::Invalid(format!("No machine type '{}' on this account.", request.host_type)))?;
        let address = free_address(&self.pool, &self.used_addresses().await)
            .ok_or_else(|| HostError::Rejected("The address pool of this account is used up.".into()))?;
        let next = self.call(reqwest::Method::GET, "/cluster/nextid", None).await?;
        let vmid: u32 = next.as_str().and_then(|s| s.parse().ok()).or_else(|| next.as_u64().map(|n| n as u32)).ok_or_else(|| HostError::Rejected("no free VM id".into()))?;

        let clone = form(&[("newid", vmid.to_string()), ("name", request.name.clone()), ("full", "1".into())]);
        let upid = self
            .call(reqwest::Method::POST, &format!("/nodes/{}/qemu/{}/clone", self.node, self.template), Some(clone))
            .await?;
        self.wait_task(upid.as_str().unwrap_or_default(), 600).await?;

        let config = form(&[
            ("cores", shape.cores.to_string()),
            ("memory", shape.memory_mb.to_string()),
            ("ipconfig0", format!("ip={address}/{},gw={}", self.prefix, self.gateway)),
            ("nameserver", self.nameserver.clone()),
            ("agent", "enabled=1".into()),
        ]);
        self.call(reqwest::Method::PUT, &format!("/nodes/{}/qemu/{vmid}/config", self.node), Some(config)).await?;
        self.call(reqwest::Method::POST, &format!("/nodes/{}/qemu/{vmid}/status/start", self.node), Some(String::new())).await?;

        // The join command runs once the machine has booted and its guest agent answers: the
        // provisioning loop calls `run_join` until the node appears (a restart of the control plane
        // between the two steps must not lose it).
        Ok(HostState { external_id: vmid.to_string(), state: "creating".into(), address: Some(address) })
    }

    async fn run_join(&self, external_id: &str, join_command: &str) -> Result<(), HostError> {
        let base = format!("/nodes/{}/qemu/{external_id}/agent", self.node);
        // The agent answers only once the guest has booted; until then Proxmox says so and the
        // caller tries again on its next pass.
        self.call(reqwest::Method::POST, &format!("{base}/ping"), Some(String::new())).await?;
        let body = form(&[("command", "bash".into()), ("command", "-c".into()), ("command", join_command.to_string())]);
        self.call(reqwest::Method::POST, &format!("{base}/exec"), Some(body)).await.map(|_| ())
    }

    async fn status(&self, external_id: &str) -> Result<HostState, HostError> {
        match self.call(reqwest::Method::GET, &format!("/nodes/{}/qemu/{external_id}/status/current", self.node), None).await {
            Ok(data) => Ok(HostState {
                external_id: external_id.into(),
                state: if data["status"].as_str() == Some("running") { "running" } else { "creating" }.into(),
                address: None,
            }),
            Err(HostError::Rejected(_)) => Ok(HostState { external_id: external_id.into(), state: "gone".into(), address: None }),
            Err(other) => Err(other),
        }
    }

    async fn destroy(&self, external_id: &str) -> Result<(), HostError> {
        let base = format!("/nodes/{}/qemu/{external_id}", self.node);
        if self.status(external_id).await?.state == "gone" {
            return Ok(());
        }
        if let Ok(upid) = self.call(reqwest::Method::POST, &format!("{base}/status/stop"), Some(String::new())).await {
            let _ = self.wait_task(upid.as_str().unwrap_or_default(), 120).await;
        }
        let upid = self.call(reqwest::Method::DELETE, &format!("{base}?purge=1&destroy-unreferenced-disks=1"), None).await?;
        self.wait_task(upid.as_str().unwrap_or_default(), 300).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_encodes_values_and_repeats_keys() {
        let body = form(&[("command", "bash".into()), ("command", "-c".into()), ("command", "curl -s \"http://x/y\" | sudo bash".into())]);
        assert!(body.starts_with("command=bash&command=-c&command=curl%20-s%20%22http%3A%2F%2Fx%2Fy%22"));
        assert!(!body.contains(' ') && !body.contains('"'));
    }

    #[test]
    fn a_pool_is_expanded_and_checked() {
        let pool = expand_pool("192.168.55.60-192.168.55.62").expect("pool");
        assert_eq!(pool, vec!["192.168.55.60", "192.168.55.61", "192.168.55.62"]);
        assert!(expand_pool("192.168.55.60").is_err());
        assert!(expand_pool("192.168.55.70-192.168.55.60").is_err());
        assert!(expand_pool("192.168.55.1-192.168.56.9").is_err());
        assert!(expand_pool("a.b.c.d-e.f.g.h").is_err());
    }

    #[test]
    fn the_first_unused_address_is_chosen() {
        let pool = expand_pool("10.0.0.5-10.0.0.7").expect("pool");
        assert_eq!(free_address(&pool, &["10.0.0.5".into()]), Some("10.0.0.6".into()));
        assert_eq!(free_address(&pool, &pool), None);
    }

    #[test]
    fn an_address_is_read_from_an_ipconfig_line() {
        assert_eq!(address_of_ipconfig("ip=192.168.55.61/24,gw=192.168.55.1"), Some("192.168.55.61".into()));
        assert_eq!(address_of_ipconfig("ip=dhcp"), Some("dhcp".into()));
        assert_eq!(address_of_ipconfig("gw=1.2.3.4"), None);
    }

    #[test]
    fn an_account_without_a_template_is_refused() {
        let config = serde_json::json!({"api_url": "https://x:8006", "node": "pve", "address_pool": "10.0.0.1-10.0.0.2", "gateway": "10.0.0.254"});
        assert!(Proxmox::from_config(&config, "t").is_err());
    }
}
