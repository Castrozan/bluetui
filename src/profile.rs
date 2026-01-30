use bluer::Address;
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct AudioProfile {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub id: AudioDeviceId,
    pub profiles: Vec<AudioProfile>,
    pub active_profile_index: Option<u32>,
}

/// Device identifier — varies by backend.
#[derive(Debug, Clone)]
pub enum AudioDeviceId {
    /// PipeWire object id (used with `wpctl set-profile <id> <index>`)
    Pipewire(u32),
    /// PulseAudio card name (used with `pactl set-card-profile <name> <profile_name>`)
    Pulseaudio(String),
}

// ── public entry points ────────────────────────────────────────────

/// Try PipeWire first, then PulseAudio.
pub fn get_audio_device(addr: &Address) -> Option<AudioDevice> {
    get_pipewire_device(addr).or_else(|| get_pulseaudio_device(addr))
}

/// Switch profile using whichever backend owns the device.
pub fn switch_profile(device: &AudioDeviceId, profile_index: u32, profile_name: &str) -> Result<String, String> {
    match device {
        AudioDeviceId::Pipewire(id) => switch_pipewire_profile(*id, profile_index),
        AudioDeviceId::Pulseaudio(card) => switch_pulseaudio_profile(card, profile_name),
    }
}

// ── PipeWire backend ───────────────────────────────────────────────

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

fn address_to_bluez_format(addr: &Address) -> String {
    addr.to_string().replace(":", "_")
}

fn get_pipewire_device(addr: &Address) -> Option<AudioDevice> {
    let output = Command::new("pw-dump").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let entries: Vec<PwDumpEntry> = serde_json::from_slice(&output.stdout).ok()?;
    let addr_str = address_to_bluez_format(addr);

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

        let profiles: Vec<AudioProfile> = params
            .enum_profile
            .iter()
            .filter(|p| p.name.as_deref() != Some("off"))
            .map(|p| AudioProfile {
                index: p.index,
                name: p.name.clone().unwrap_or_default(),
                description: p.description.clone().unwrap_or_default(),
                available: p.available.as_deref() == Some("yes"),
            })
            .filter(|p| p.available)
            .collect();

        let active_profile_index = params.profile.first().map(|p| p.index);

        return Some(AudioDevice {
            id: AudioDeviceId::Pipewire(entry.id),
            profiles,
            active_profile_index,
        });
    }

    None
}

fn switch_pipewire_profile(device_id: u32, profile_index: u32) -> Result<String, String> {
    let output = Command::new("wpctl")
        .args([
            "set-profile",
            &device_id.to_string(),
            &profile_index.to_string(),
        ])
        .output()
        .map_err(|e| format!("Failed to run wpctl: {e}"))?;

    if output.status.success() {
        Ok("Profile switched".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("wpctl failed: {stderr}"))
    }
}

// ── PulseAudio backend ─────────────────────────────────────────────

fn get_pulseaudio_device(addr: &Address) -> Option<AudioDevice> {
    let output = Command::new("pactl")
        .args(["--format=json", "list", "cards"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let cards: Vec<PaCard> = serde_json::from_slice(&output.stdout).ok()?;
    let addr_str = address_to_bluez_format(addr);

    for card in &cards {
        // Match by bluez5 address in properties
        let card_addr = card
            .properties
            .get("api.bluez5.address")
            .or_else(|| card.properties.get("device.string"))
            .map(|s| s.replace(":", "_"));

        // Also try matching via card name (bluez_card.XX_XX_XX_XX_XX_XX)
        let name_matches = card.name.contains(&addr_str);
        let addr_matches = card_addr.as_deref() == Some(&addr_str);

        if !name_matches && !addr_matches {
            continue;
        }

        let mut profiles: Vec<AudioProfile> = Vec::new();
        let mut active_profile_index: Option<u32> = None;

        for (idx, pa_profile) in card.profiles.iter().enumerate() {
            if pa_profile.name == "off" {
                continue;
            }
            let available = pa_profile.available;
            let profile = AudioProfile {
                index: idx as u32,
                name: pa_profile.name.clone(),
                description: pa_profile.description.clone(),
                available,
            };
            if available {
                profiles.push(profile);
            }
        }

        // Find active profile
        if let Some(ref active_name) = card.active_profile {
            for (idx, p) in profiles.iter().enumerate() {
                if p.name == *active_name {
                    active_profile_index = Some(idx as u32);
                    break;
                }
            }
        }

        if profiles.is_empty() {
            continue;
        }

        return Some(AudioDevice {
            id: AudioDeviceId::Pulseaudio(card.name.clone()),
            profiles,
            active_profile_index,
        });
    }

    None
}

#[derive(Deserialize)]
struct PaCard {
    #[serde(default)]
    name: String,
    #[serde(default)]
    properties: std::collections::HashMap<String, String>,
    #[serde(default)]
    profiles: Vec<PaProfile>,
    #[serde(rename = "active_profile", default)]
    active_profile: Option<String>,
}

#[derive(Deserialize)]
struct PaProfile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    available: bool,
}

fn switch_pulseaudio_profile(card_name: &str, profile_name: &str) -> Result<String, String> {
    let output = Command::new("pactl")
        .args(["set-card-profile", card_name, profile_name])
        .output()
        .map_err(|e| format!("Failed to run pactl: {e}"))?;

    if output.status.success() {
        Ok("Profile switched".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("pactl failed: {stderr}"))
    }
}
