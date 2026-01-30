use bluer::Address;
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct PipewireProfile {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct PipewireDevice {
    pub id: u32,
    pub profiles: Vec<PipewireProfile>,
    pub active_profile_index: Option<u32>,
}

#[derive(Deserialize)]
struct PwDumpEntry {
    id: u32,
    #[serde(default)]
    info: Option<PwInfo>,
}

#[derive(Deserialize)]
struct PwInfo {
    #[serde(default)]
    props: Option<PwProps>,
    #[serde(default)]
    params: Option<PwParams>,
}

#[derive(Deserialize)]
struct PwProps {
    #[serde(rename = "api.bluez5.address")]
    bluez5_address: Option<String>,
}

#[derive(Deserialize)]
struct PwParams {
    #[serde(rename = "EnumProfile", default)]
    enum_profile: Vec<PwEnumProfile>,
    #[serde(rename = "Profile", default)]
    profile: Vec<PwActiveProfile>,
}

#[derive(Deserialize)]
struct PwEnumProfile {
    index: u32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    available: Option<String>,
}

#[derive(Deserialize)]
struct PwActiveProfile {
    index: u32,
}

fn address_to_pw_format(addr: &Address) -> String {
    addr.to_string().replace(":", "_")
}

pub fn get_pipewire_device(addr: &Address) -> Option<PipewireDevice> {
    let output = Command::new("pw-dump").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let entries: Vec<PwDumpEntry> = serde_json::from_slice(&output.stdout).ok()?;
    let addr_str = address_to_pw_format(addr);

    for entry in &entries {
        let Some(info) = &entry.info else { continue };
        let Some(props) = &info.props else { continue };
        let Some(ref bluez_addr) = props.bluez5_address else {
            continue;
        };

        if bluez_addr.replace(":", "_") != addr_str {
            continue;
        }

        let Some(params) = &info.params else { continue };
        if params.enum_profile.is_empty() {
            continue;
        }

        let profiles: Vec<PipewireProfile> = params
            .enum_profile
            .iter()
            .filter(|p| p.name.as_deref() != Some("off"))
            .map(|p| PipewireProfile {
                index: p.index,
                name: p.name.clone().unwrap_or_default(),
                description: p.description.clone().unwrap_or_default(),
                available: p.available.as_deref() == Some("yes"),
            })
            .filter(|p| p.available)
            .collect();

        let active_profile_index = params.profile.first().map(|p| p.index);

        return Some(PipewireDevice {
            id: entry.id,
            profiles,
            active_profile_index,
        });
    }

    None
}

pub fn switch_profile(device_id: u32, profile_index: u32) -> Result<String, String> {
    let output = Command::new("wpctl")
        .args(["set-profile", &device_id.to_string(), &profile_index.to_string()])
        .output()
        .map_err(|e| format!("Failed to run wpctl: {e}"))?;

    if output.status.success() {
        Ok("Profile switched".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("wpctl failed: {stderr}"))
    }
}
